import os
import queue
import signal
import socket
import subprocess
import sys
import threading
import time
import wave

import history
import mlx_whisper
import numpy as np
import Quartz
import sounddevice as sd
from state_machine import Action, DictationStateMachine, State

DEBUG_AUDIO_DIR = "/tmp/voicerec_debug"

# ─── Configuration ────────────────────────────────────────────────────────────

MODEL_SIZE = "large-v3-turbo"
SAMPLE_RATE = 16000
BLOCK_SIZE = 1024

# ─── State ────────────────────────────────────────────────────────────────────

_recording = False
_audio_queue = queue.Queue()
_audio_chunks = []
_audio_done = threading.Event()
_lock = threading.Lock()

_indicator_proc: subprocess.Popen | None = None
_rms_smoothed = 0.0

# ─── Hotkey (CGEventTap) ─────────────────────────────────────────────────────

VK_C = 8
VK_V = 9
VK_ESC = 53
HOTKEY_FLAGS = Quartz.kCGEventFlagMaskAlternate | Quartz.kCGEventFlagMaskShift

_sm = DictationStateMachine()


class HotkeyManager:
    def __init__(self):
        self._tap = None
        self._source = None
        self._consumed_down = False

    def _callback(self, proxy, event_type, event, refcon):
        if event_type not in (Quartz.kCGEventKeyDown, Quartz.kCGEventKeyUp):
            return event

        key_code = Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventKeycode)
        flags = Quartz.CGEventGetFlags(event)
        is_repeat = Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventAutorepeat)
        ts = time.monotonic()

        if key_code == VK_ESC and event_type == Quartz.kCGEventKeyDown:
            # 無鎖讀取：極端情況下 Esc 與熱鍵同時按、state 尚未更新時會放行，
            # 使用者再按一次即可，不值得為此在 tap callback 內上鎖。
            if _sm.state is not State.IDLE:
                _events.put(("esc", ts))
                return None
            return event

        if key_code != VK_C:
            return event

        if event_type == Quartz.kCGEventKeyDown:
            if (flags & HOTKEY_FLAGS) == HOTKEY_FLAGS and not is_repeat:
                self._consumed_down = True
                _events.put(("down", ts))
                return None
            if is_repeat and self._consumed_down:
                return None
            return event

        # keyUp：只要 keyDown 是我們吃掉的，keyUp 一律吃掉（不管修飾鍵還在不在）
        if self._consumed_down:
            self._consumed_down = False
            _events.put(("up", ts))
            return None
        return event

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
_cancel_processing = threading.Event()
_events: queue.Queue = queue.Queue()


def _event_dispatcher():
    """單一執行緒依鍵盤事件原始順序驅動狀態機。

    tap callback 只負責入列；所有狀態轉移都在這裡序列化，
    避免 keyDown/keyUp 各自開 thread 導致亂序（快速點放時 keyUp 先到會卡死錄音）。
    重活（轉錄）另開 thread，不堵住後續事件。
    """
    while True:
        kind, ts = _events.get()
        if kind == "down":
            _on_hotkey_down(ts)
        elif kind == "up":
            _on_hotkey_up(ts)
        elif kind == "esc":
            _on_esc()


def _start_processing():
    """每個處理 session 配發自己的取消旗標。

    舊 session 的轉錄執行緒持有自己的 Event；新 session 換新的一顆，
    不會把前一段還沒檢查到的 Esc 取消洗掉。
    """
    global _cancel_processing
    cancel = threading.Event()
    _cancel_processing = cancel
    threading.Thread(target=_do_stop_and_process, args=(cancel,), daemon=True).start()


def _on_hotkey_down(ts: float):
    with _lock:
        action = _sm.hotkey_down(ts)
    if action is Action.START_RECORDING:
        _do_start_recording()
    elif action is Action.STOP_AND_PROCESS:
        _start_processing()


def _on_hotkey_up(ts: float):
    with _lock:
        action = _sm.hotkey_up(ts)
    if action is Action.ENTER_HANDSFREE:
        _indicator_send("handsfree")
        print("🔁 Hands-free mode (press hotkey again to stop)", flush=True)
    elif action is Action.STOP_AND_PROCESS:
        _start_processing()


def _on_esc():
    with _lock:
        action = _sm.esc()
    if action is Action.CANCEL_RECORDING:
        threading.Thread(target=_do_cancel_recording, daemon=True).start()
    elif action is Action.CANCEL_PROCESSING:
        _cancel_processing.set()
        _indicator_send("cancel")
        print("✕ Cancelled (processing result will go to history only)", flush=True)


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
    global _rms_smoothed
    if status:
        print(f"[audio] {status}", file=sys.stderr)
    rms = float(np.sqrt(np.mean(indata ** 2)))
    _rms_smoothed = _rms_smoothed * 0.6 + rms * 0.4
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


def _audio_level_poller():
    while _recording:
        _indicator_send(f"level {_rms_smoothed:.4f}")
        time.sleep(0.03)


def _do_start_recording():
    global _recording, _audio_chunks, _audio_queue, _rms_smoothed
    _recording = True
    _rms_smoothed = 0.0
    _audio_chunks = []
    _audio_queue = queue.Queue()

    threading.Thread(target=_record_worker, daemon=True).start()
    threading.Thread(target=_audio_level_poller, daemon=True).start()
    print("🎤 Recording... (release to paste, Esc to cancel)", flush=True)

    _indicator_send("recording")


def _rms(audio: np.ndarray) -> float:
    return float(np.sqrt(np.mean(audio ** 2)))


def _normalize_audio(audio: np.ndarray) -> np.ndarray:
    peak = np.max(np.abs(audio))
    if peak > 0.001:
        audio = audio / peak * 0.95
    return audio


import re as _re


def _clean_transcript(result: dict) -> str:
    text = result["text"].strip()
    text = _re.sub(r"(?<=[\u4e00-\u9fff]) +(?=[\u4e00-\u9fff])", "", text)
    text = _re.sub(r" +(?=[\u3000-\u303f\uff00-\uffef\u2000-\u206f])", "", text)
    text = _re.sub(r"(?<=[\u3000-\u303f\uff00-\uffef\u2000-\u206f]) +", "", text)
    text = _re.sub(r" +", " ", text)
    return text.strip()


def _collect_audio() -> np.ndarray | None:
    """停止錄音執行緒並收集音訊；不合格回傳 None 並顯示 error 態。"""
    global _recording
    _recording = False

    if not _audio_done.wait(timeout=2.0):
        print("Audio capture timed out", flush=True)
        _indicator_send("error")
        history.append_entry(raw="", text="", duration_s=0, status="error")
        return None
    if not _audio_chunks:
        _indicator_send("error")
        history.append_entry(raw="", text="", duration_s=0, status="error")
        return None

    audio = np.concatenate(_audio_chunks)
    dur = len(audio) / SAMPLE_RATE
    if dur < 0.3:
        print("Audio too short, ignoring", flush=True)
        _indicator_send("error")
        history.append_entry(raw="", text="", duration_s=dur, status="too_short")
        return None
    if _rms(audio) < 0.01:
        print("  ⛔ RMS too low — silence, ignoring", flush=True)
        _indicator_send("error")
        history.append_entry(raw="", text="", duration_s=dur, status="silent")
        return None

    _save_debug_audio(audio, "rec")
    return _normalize_audio(audio)


def _transcribe(audio: np.ndarray) -> str:
    result = mlx_whisper.transcribe(
        audio,
        path_or_hf_repo=f"mlx-community/whisper-{MODEL_SIZE}",
        language="zh",
    )
    return _clean_transcript(result)


def _do_stop_and_process(cancel: threading.Event):
    try:
        audio = _collect_audio()
        if audio is None:
            return
        dur = len(audio) / SAMPLE_RATE

        _indicator_send("processing")
        print("⏳ Transcribing...", flush=True)
        try:
            raw = _transcribe(audio)
        except Exception as e:
            print(f"Transcription failed: {e}", flush=True)
            _indicator_send("error")
            history.append_entry(raw="", text="", duration_s=dur, status="error")
            return

        if not raw:
            _indicator_send("error")
            history.append_entry(raw="", text="", duration_s=dur, status="silent")
            return

        text = _apply_dict(raw)
        if _llm_available:
            print("  ✨ Refining with LLM...", flush=True)
            text = _llm_refine(text, context=_get_window_context())  # 失敗時內部回傳原文

        if cancel.is_set():
            history.append_entry(raw=raw, text=text, duration_s=dur, status="cancelled")
            return

        print(f"📝 {text}", flush=True)
        _paste_text(text)
        _indicator_send("success")
        history.append_entry(raw=raw, text=text, duration_s=dur, status="pasted")
    finally:
        with _lock:
            _sm.processing_finished()


def _do_cancel_recording():
    """Esc 取消錄音：立即收 UI，背景仍轉錄一份進歷史（可救回）。"""
    _indicator_send("cancel")
    audio = None
    global _recording
    _recording = False
    if _audio_done.wait(timeout=2.0) and _audio_chunks:
        audio = np.concatenate(_audio_chunks)
    if audio is None or len(audio) < SAMPLE_RATE * 0.3 or _rms(audio) < 0.01:
        dur = 0 if audio is None else len(audio) / SAMPLE_RATE
        history.append_entry(raw="", text="", duration_s=dur, status="cancelled")
        return
    dur = len(audio) / SAMPLE_RATE

    def _bg():
        try:
            raw = _transcribe(_normalize_audio(audio))
        except Exception:
            raw = ""
        history.append_entry(raw=raw, text=raw, duration_s=dur, status="cancelled")

    threading.Thread(target=_bg, daemon=True).start()


def _paste_text(text: str):
    """備份剪貼簿 → 寫入文字 → CGEvent 送 Cmd+V → 還原剪貼簿。

    游標在哪個輸入框，字就貼在哪。不點滑鼠、不搶焦點。
    還原只保留純文字內容（夠用且簡單）。
    """
    try:
        old = subprocess.run(["pbpaste"], capture_output=True).stdout
    except Exception as e:
        print(f"Clipboard backup failed: {e}", flush=True)
        old = None
    try:
        subprocess.run(["pbcopy"], input=text.encode("utf-8"))
        time.sleep(0.05)

        src = Quartz.CGEventSourceCreate(Quartz.kCGEventSourceStateHIDSystemState)
        down = Quartz.CGEventCreateKeyboardEvent(src, VK_V, True)
        up = Quartz.CGEventCreateKeyboardEvent(src, VK_V, False)
        Quartz.CGEventSetFlags(down, Quartz.kCGEventFlagMaskCommand)
        Quartz.CGEventSetFlags(up, Quartz.kCGEventFlagMaskCommand)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, down)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, up)
    except Exception as e:
        print(f"Paste failed: {e}", flush=True)
    finally:
        if old is not None:
            time.sleep(0.3)
            try:
                subprocess.run(["pbcopy"], input=old)
            except Exception:
                pass


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
    if "--history" in sys.argv:
        for e in history.read_recent(20):
            print(f"[{e['ts']}] ({e['status']}, {e['duration_s']}s) {e['text'] or e['raw']}")
        return

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
    print("  Hotkey: hold Option+Shift+C to talk, release to paste", flush=True)
    print("  Quick tap = hands-free mode; Esc = cancel", flush=True)
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

    threading.Thread(target=_event_dispatcher, daemon=True).start()
    print("👂  Listening for hotkey 'Option+Shift+C'...", flush=True)
    _hotkey_mgr.start()


if __name__ == "__main__":
    main()
