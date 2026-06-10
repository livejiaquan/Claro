import os
import queue
import signal
import socket
import subprocess
import sys
import threading
import time
import wave

import mlx_whisper
import numpy as np
import pyautogui
import Quartz
import sounddevice as sd

DEBUG_AUDIO_DIR = "/tmp/voicerec_debug"
MENU_RESULT = "/tmp/mic_indicator_result"

# ─── Configuration ────────────────────────────────────────────────────────────

MODEL_SIZE = "large-v3-mlx"
SAMPLE_RATE = 16000
BLOCK_SIZE = 1024

# ─── State ────────────────────────────────────────────────────────────────────

_recording = False
_audio_queue = queue.Queue()
_audio_chunks = []
_audio_done = threading.Event()
_lock = threading.Lock()

_indicator_proc: subprocess.Popen | None = None

# ─── Hotkey (CGEventTap) ─────────────────────────────────────────────────────

VK_C = 8
HOTKEY_FLAGS = Quartz.kCGEventFlagMaskAlternate | Quartz.kCGEventFlagMaskShift


class HotkeyManager:
    def __init__(self):
        self._tap = None
        self._source = None
        self._hotkey_down = False

    def _callback(self, proxy, event_type, event, refcon):
        if event_type not in (Quartz.kCGEventKeyDown, Quartz.kCGEventKeyUp):
            return event

        key_code = Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventKeycode)
        flags = Quartz.CGEventGetFlags(event)

        is_target = (
            key_code == VK_C
            and (flags & HOTKEY_FLAGS) == HOTKEY_FLAGS
        )

        if not is_target:
            self._hotkey_down = False
            return event

        if event_type == Quartz.kCGEventKeyDown and not self._hotkey_down:
            self._hotkey_down = True
            threading.Thread(target=_on_hotkey, daemon=True).start()

        if event_type == Quartz.kCGEventKeyUp:
            self._hotkey_down = False

        return None

    def start(self):
        event_mask = (1 << Quartz.kCGEventKeyDown) | (1 << Quartz.kCGEventKeyUp)
        self._tap = Quartz.CGEventTapCreate(
            Quartz.kCGSessionEventTap,
            Quartz.kCGHeadInsertEventTap,
            Quartz.kCGEventTapOptionDefault,
            event_mask,
            self._callback,
            None,
        )

        if self._tap is None:
            raise RuntimeError(
                "CGEventTap creation failed.\n"
                "  Make sure Accessibility permission is GRANTED.\n"
                "  → System Settings → Privacy & Security → Accessibility"
            )

        self._source = Quartz.CFMachPortCreateRunLoopSource(None, self._tap, 0)
        Quartz.CFRunLoopAddSource(
            Quartz.CFRunLoopGetCurrent(),
            self._source,
            Quartz.kCFRunLoopDefaultMode,
        )
        Quartz.CFRunLoopRun()

    def stop(self):
        if self._source is not None:
            Quartz.CFRunLoopRemoveSource(
                Quartz.CFRunLoopGetCurrent(),
                self._source,
                Quartz.kCFRunLoopDefaultMode,
            )
        if self._tap is not None:
            Quartz.CFMachPortInvalidate(self._tap)
            self._tap = None
        Quartz.CFRunLoopStop(Quartz.CFRunLoopGetCurrent())


_hotkey_mgr = HotkeyManager()


def _on_hotkey():
    global _recording
    with _lock:
        if not _recording:
            _do_start_recording()
        else:
            _do_stop_and_transcribe()


# ─── Audio (debug) ─────────────────────────────────────────────────────────────

_AUDIO_DEBUG_ENABLED = False  # toggle to True for WAV dumps


def _enable_audio_debug():
    global _AUDIO_DEBUG_ENABLED
    _AUDIO_DEBUG_ENABLED = True
    os.makedirs(DEBUG_AUDIO_DIR, exist_ok=True)
    print(f"  Audio debug enabled → {DEBUG_AUDIO_DIR}", flush=True)


def _save_debug_audio(audio: np.ndarray, label: str):
    if not _AUDIO_DEBUG_ENABLED:
        return
    path = os.path.join(DEBUG_AUDIO_DIR, f"{label}_{int(time.time())}.wav")
    try:
        audio_int16 = (np.clip(audio, -1, 1) * 32767).astype(np.int16)
        with wave.open(path, "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(SAMPLE_RATE)
            wf.writeframes(audio_int16.tobytes())
        print(f"  💾 Debug WAV: {path} ({len(audio)/SAMPLE_RATE:.1f}s)", flush=True)
    except Exception as e:
        print(f"  Failed to save debug audio: {e}", flush=True)


def _print_audio_devices():
    try:
        devices = sd.query_devices()
        print("  Available input devices:", flush=True)
        for i, dev in enumerate(devices):
            if dev["max_input_channels"] > 0:
                mark = " ← default" if i == sd.default.device[0] else ""
                print(f"    [{i}] {dev['name']}{mark}", flush=True)
        default_input = sd.default.device[0]
        if default_input is not None:
            print(f"  Using device: [{default_input}] {devices[default_input]['name']}", flush=True)
        else:
            print("  No default input device configured", flush=True)
        print(f"  Sample rate: {SAMPLE_RATE}, Block size: {BLOCK_SIZE}", flush=True)
    except Exception as e:
        print(f"  Could not query audio devices: {e}", flush=True)


# ─── Mic calibration ───────────────────────────────────────────────────────────

def _calibrate_mic():
    print("  🔍 Recording 1s sample to check mic level...", flush=True)
    try:
        sample = sd.rec(int(SAMPLE_RATE), samplerate=SAMPLE_RATE, channels=1, dtype="float32")
        sd.wait()
        rms = _rms(sample.flatten())
        if rms < 0.01:
            print(f"  ⚠️  Mic level VERY LOW (RMS={rms:.4f}). "
                  f"Check your input device or speak louder.", flush=True)
        else:
            print(f"  ✅ Mic level OK (RMS={rms:.4f})", flush=True)
    except Exception as e:
        print(f"  Mic calibration failed: {e}", flush=True)


# ─── Audio ────────────────────────────────────────────────────────────────────

def _audio_callback(indata, frames, time_info, status):
    if status:
        print(f"[audio] {status}", file=sys.stderr)
    _audio_queue.put(indata.copy())


def _record_worker():
    _audio_done.clear()
    with sd.InputStream(
        samplerate=SAMPLE_RATE,
        channels=1,
        dtype="float32",
        blocksize=BLOCK_SIZE,
        callback=_audio_callback,
    ):
        while _recording:
            sd.sleep(50)

    while not _audio_queue.empty():
        _audio_chunks.append(_audio_queue.get().flatten())

    _audio_done.set()


def _do_start_recording():
    global _recording, _audio_chunks, _audio_queue
    _recording = True
    _audio_chunks = []
    _audio_queue = queue.Queue()

    threading.Thread(target=_record_worker, daemon=True).start()
    print("🎤 Recording... (press Option+Shift+C again to stop)", flush=True)

    mx, my = pyautogui.position()
    try:
        os.unlink(MENU_RESULT)
    except FileNotFoundError:
        pass
    _indicator_send(f"recording {mx} {my}")


def _rms(audio: np.ndarray) -> float:
    return float(np.sqrt(np.mean(audio ** 2)))


def _do_stop_and_transcribe():
    global _recording
    _recording = False
    _indicator_send("hide")

    if not _audio_done.wait(timeout=2.0):
        print("Audio capture timed out", flush=True)
        return

    if not _audio_chunks:
        print("No audio captured", flush=True)
        return

    audio = np.concatenate(_audio_chunks)

    if len(audio) < SAMPLE_RATE * 0.3:
        print("Audio too short, ignoring", flush=True)
        return

    rms = _rms(audio)
    dur = len(audio) / SAMPLE_RATE
    print(f"  Audio: {dur:.1f}s, RMS={rms:.4f}", flush=True)
    _save_debug_audio(audio, "rec")

    if rms < 0.02:
        print("  ⛔ RMS too low — likely silence/noise, ignoring", flush=True)
        return

    print("⏳ Transcribing...", flush=True)

    try:
        result = mlx_whisper.transcribe(
            audio,
            path_or_hf_repo=f"mlx-community/whisper-{MODEL_SIZE}",
        )
        text = result["text"].strip()
    except Exception as e:
        print(f"Transcription failed: {e}", flush=True)
        return

    if not text:
        print("No speech detected", flush=True)
        return

    text = _apply_dict(text)

    if _llm_available:
        ctx = _get_window_context()
        print("  ✨ Refining with LLM...", flush=True)
        text = _llm_refine(text, context=ctx)

    print(f"📝 {text}", flush=True)

    mx, my = pyautogui.position()
    _indicator_send(f"menu {mx} {my}")
    choice = _wait_menu_choice(timeout=15)
    if choice == "confirm":
        _type_text_at_mouse(text)
    else:
        print("  Cancelled", flush=True)


def _type_text_at_mouse(text: str):
    try:
        x, y = pyautogui.position()
        pyautogui.click(x, y)
        time.sleep(0.05)
        proc = subprocess.Popen(["pbcopy"], stdin=subprocess.PIPE)
        proc.communicate(text.encode("utf-8"))
        pyautogui.hotkey("command", "v")
    except Exception as e:
        print(f"Failed to type text: {e}", flush=True)


def _wait_menu_choice(timeout: float = 15) -> str:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with open(MENU_RESULT) as f:
                choice = f.read().strip()
            os.unlink(MENU_RESULT)
            return choice if choice in ("confirm", "cancel") else "confirm"
        except FileNotFoundError:
            pass
        time.sleep(0.1)
    return "confirm"


# ─── Mic indicator (Swift overlay) ───────────────────────────────────────────

INDICATOR_SOCKET = "/tmp/mic_indicator.sock"


def _start_indicator():
    global _indicator_proc
    paths = [
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "mic_indicator"),
        "/usr/local/bin/mic_indicator",
    ]
    for p in paths:
        if os.path.exists(p):
            _indicator_proc = subprocess.Popen(
                [p],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return
    print("mic_indicator not found, skipping floating overlay", flush=True)


def _indicator_send(command: str):
    try:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(0.5)
        sock.connect(INDICATOR_SOCKET)
        sock.sendall((command + "\n").encode())
        sock.close()
    except Exception:
        pass


def _stop_indicator():
    if _indicator_proc and _indicator_proc.poll() is None:
        _indicator_send("quit")
        try:
            _indicator_proc.wait(timeout=2)
        except Exception:
            _indicator_proc.kill()


# ─── Personal Dictionary ───────────────────────────────────────────────────────

PERSONAL_DICT: dict[str, str] = {
    "GBT": "GPT",
    "My Torch": "PyTorch",
}


def _apply_dict(text: str) -> str:
    for wrong, right in PERSONAL_DICT.items():
        text = text.replace(wrong, right)
        text = text.replace(wrong.lower(), right.lower())
    return text


# ─── Screen context ───────────────────────────────────────────────────────────

def _get_window_context() -> str:
    """Read foreground app + input field context via macOS Accessibility API."""
    parts = []
    try:
        import AppKit
        from ApplicationServices import (
            AXUIElementCopyAttributeValue,
            AXUIElementCreateApplication,
        )
        _AXV = lambda el, attr: AXUIElementCopyAttributeValue(el, attr, None)

        ws = AppKit.NSWorkspace.sharedWorkspace()
        front = ws.frontmostApplication()
        parts.append(f"App: {front.localizedName()}")

        pid = front.processIdentifier()
        app_elem = AXUIElementCreateApplication(pid)

        err, window = _AXV(app_elem, "AXFocusedWindow")
        if err == 0:
            err, title = _AXV(window, "AXTitle")
            if err == 0 and title:
                parts.append(f"Window: {title}")

        err, focused = _AXV(app_elem, "AXFocusedUIElement")
        if err == 0:
            err, selected = _AXV(focused, "AXSelectedText")
            if err == 0 and isinstance(selected, str) and selected.strip():
                parts.append(f"Selected: {selected.strip()[:200]}")
    except Exception:
        pass

    return "\n".join(parts) if parts else ""


# ─── LLM post-processing ──────────────────────────────────────────────────────

_llm_model = None
_llm_tokenizer = None
_llm_available = False
LLM_MODEL_ID = "mlx-community/Qwen2.5-1.5B-Instruct-4bit"


def _init_llm():
    global _llm_model, _llm_tokenizer, _llm_available
    try:
        import mlx_lm

        print(f"Loading LLM '{LLM_MODEL_ID}'...", flush=True)
        t0 = time.time()
        _llm_model, _llm_tokenizer = mlx_lm.load(LLM_MODEL_ID)
        print(f"  LLM loaded ({time.time() - t0:.1f}s)", flush=True)
        _llm_available = True
    except Exception as e:
        print(f"  LLM not available, skipping post-processing: {e}", flush=True)
        _llm_available = False


def _llm_refine(text: str, context: str = "") -> str:
    if not _llm_available:
        return text

    ctx_block = f"\nContext:\n{context}\n\n" if context else "\n"
    system_prompt = (
        "Clean up this dictation:\n"
        "- Fix transcription errors using context when available\n"
        "- Remove filler words (嗯, 啊, 那個, um, uh, like)\n"
        "- If speaker corrects themselves (e.g. '三點 不對 四點'), keep only the final version\n"
        "- Add punctuation\n"
        "- Use Traditional Chinese (繁體中文), not Simplified\n"
        "Output ONLY the cleaned text."
    )

    messages = [
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": f"{ctx_block}{text}"},
    ]

    try:
        import mlx_lm

        prompt = _llm_tokenizer.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=True
        )
        response = mlx_lm.generate(
            _llm_model,
            _llm_tokenizer,
            prompt=prompt,
            max_tokens=512,
        )
        cleaned = response.strip()
        return cleaned if cleaned else text
    except Exception as e:
        print(f"  LLM refinement failed: {e}", flush=True)
        return text


# ─── Preload ──────────────────────────────────────────────────────────────────

def _preload_model():
    print(f"Loading model 'mlx-community/whisper-{MODEL_SIZE}'...", flush=True)
    t0 = time.time()
    mlx_whisper.transcribe(
        np.zeros(SAMPLE_RATE, dtype=np.float32),
        path_or_hf_repo=f"mlx-community/whisper-{MODEL_SIZE}",
    )
    print(f"Model loaded ({time.time() - t0:.1f}s)", flush=True)


# ─── Permission check ─────────────────────────────────────────────────────────

def _check_accessibility():
    from HIServices import AXIsProcessTrusted

    trusted = AXIsProcessTrusted()
    if trusted:
        print("✅  Accessibility permission: GRANTED", flush=True)
    else:
        print("❌  Accessibility permission: NOT GRANTED", flush=True)
        print("  → Go to System Settings → Privacy & Security → Accessibility,", flush=True)
        print("    add your terminal app, restart it, and try again.", flush=True)


# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    _custom_terms = []
    args = [a for a in sys.argv[1:] if not a.startswith("--add-term")]
    for a in sys.argv[1:]:
        if a.startswith("--add-term="):
            part = a.split("=", 1)[1]
            if ":" in part:
                wrong, right = part.split(":", 1)
                PERSONAL_DICT[wrong.strip()] = right.strip()
                print(f"  📖 Dict: '{wrong.strip()}' → '{right.strip()}'", flush=True)

    if "--debug" in sys.argv:
        _enable_audio_debug()

    print("=" * 50, flush=True)
    print("  Voice-to-Text Tool", flush=True)
    print(f"  Model: whisper-{MODEL_SIZE}", flush=True)
    print("  Hotkey: Option + Shift + C", flush=True)
    print("  Press once to start recording, again to transcribe", flush=True)
    print("=" * 50, flush=True)

    _check_accessibility()
    _print_audio_devices()
    _calibrate_mic()
    _start_indicator()
    _preload_model()
    _init_llm()

    def _cleanup(*_):
        _hotkey_mgr.stop()
        _stop_indicator()
        sys.exit(0)

    signal.signal(signal.SIGINT, _cleanup)
    signal.signal(signal.SIGTERM, _cleanup)

    print("👂  Listening for hotkey 'Option+Shift+C'...", flush=True)
    _hotkey_mgr.start()


if __name__ == "__main__":
    main()
