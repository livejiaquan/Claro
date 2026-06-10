import importlib
import sys
import time
import types

import numpy as np


def wait_for(predicate, timeout=1.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.005)
    return False


def load_main(monkeypatch):
    sys.modules.pop("main", None)

    fake_whisper = types.SimpleNamespace(transcribe=lambda *args, **kwargs: {"text": ""})
    fake_sounddevice = types.SimpleNamespace()
    fake_quartz = types.SimpleNamespace(
        kCGEventFlagMaskAlternate=1 << 19,
        kCGEventFlagMaskShift=1 << 17,
        kCGEventFlagMaskCommand=1 << 20,
        kCGEventKeyDown=10,
        kCGEventKeyUp=11,
        kCGKeyboardEventKeycode="keycode",
        kCGKeyboardEventAutorepeat="autorepeat",
        kCGSessionEventTap=1,
        kCGHeadInsertEventTap=0,
        kCGEventTapOptionDefault=0,
        kCGEventSourceStateHIDSystemState=0,
        kCGHIDEventTap=0,
        kCFRunLoopDefaultMode=0,
    )
    fake_quartz.CGEventGetIntegerValueField = lambda event, field: event.get(field, 0)
    fake_quartz.CGEventGetFlags = lambda event: event.get("flags", 0)
    fake_quartz.CGEventSourceCreate = lambda *_: object()
    fake_quartz.CGEventCreateKeyboardEvent = lambda *_: object()
    fake_quartz.CGEventSetFlags = lambda *_: None
    fake_quartz.CGEventPost = lambda *_: None
    fake_quartz.CGEventTapCreate = lambda *_: object()
    fake_quartz.CFMachPortCreateRunLoopSource = lambda *_: object()
    fake_quartz.CFRunLoopAddSource = lambda *_: None
    fake_quartz.CFRunLoopGetCurrent = lambda: object()
    fake_quartz.CFRunLoopRun = lambda: None
    fake_quartz.CFRunLoopRemoveSource = lambda *_: None
    fake_quartz.CFMachPortInvalidate = lambda *_: None
    fake_quartz.CFRunLoopStop = lambda *_: None

    monkeypatch.setitem(sys.modules, "mlx_whisper", fake_whisper)
    monkeypatch.setitem(sys.modules, "sounddevice", fake_sounddevice)
    monkeypatch.setitem(sys.modules, "Quartz", fake_quartz)
    return importlib.import_module("main")


def test_task4_removes_confirm_flow_and_uses_turbo(monkeypatch):
    main = load_main(monkeypatch)

    assert main.MODEL_SIZE == "large-v3-turbo"
    assert not hasattr(main, "MENU_RESULT")
    assert not hasattr(main, "_wait_menu_choice")
    assert not hasattr(main, "_on_hotkey")
    assert hasattr(main, "_on_hotkey_down")
    assert hasattr(main, "_on_hotkey_up")
    assert hasattr(main, "_on_esc")


def test_hotkey_handlers_drive_recording_handsfree_processing_and_esc(monkeypatch):
    main = load_main(monkeypatch)
    calls = []

    monkeypatch.setattr(main, "_do_start_recording", lambda: calls.append("start"))
    monkeypatch.setattr(main, "_do_stop_and_process", lambda cancel: calls.append("process"))
    monkeypatch.setattr(main, "_do_cancel_recording", lambda: calls.append("cancel_recording"))
    monkeypatch.setattr(main, "_indicator_send", lambda command: calls.append(command))

    # process/cancel_recording 由事件處理函式另開 thread 執行，需等待
    main._sm = main.DictationStateMachine()
    main._on_hotkey_down(10.0)
    main._on_hotkey_up(10.1)
    main._on_hotkey_down(12.0)
    assert wait_for(lambda: calls == ["start", "handsfree", "process"]), calls

    calls.clear()
    main._sm = main.DictationStateMachine()
    main._on_hotkey_down(20.0)
    main._on_esc()
    assert wait_for(lambda: calls == ["start", "cancel_recording"]), calls
    assert main._sm.state is main.State.IDLE

    calls.clear()
    main._sm = main.DictationStateMachine()
    main._on_hotkey_down(30.0)
    main._on_hotkey_up(31.0)
    assert wait_for(lambda: calls == ["start", "process"]), calls
    main._on_esc()
    assert wait_for(lambda: calls == ["start", "process", "cancel"]), calls
    assert main._cancel_processing.is_set()
    assert main._sm.state is main.State.IDLE


def test_stop_and_process_pastes_success_and_records_history(monkeypatch):
    import threading

    main = load_main(monkeypatch)
    audio = np.ones(main.SAMPLE_RATE, dtype=np.float32)
    sent = []
    pasted = []
    entries = []

    main._sm = main.DictationStateMachine()
    main._sm.hotkey_down(1.0)
    main._sm.hotkey_up(2.0)
    monkeypatch.setattr(main, "_collect_audio", lambda: audio)
    monkeypatch.setattr(main, "_transcribe", lambda _: "GBT 測試")
    monkeypatch.setattr(main, "_indicator_send", lambda command: sent.append(command))
    monkeypatch.setattr(main, "_paste_text", lambda text: pasted.append(text))
    monkeypatch.setattr(main.history, "append_entry", lambda **entry: entries.append(entry))
    monkeypatch.setattr(main, "_llm_available", False)

    main._do_stop_and_process(threading.Event())

    assert sent == ["processing", "success"]
    assert pasted == ["GPT 測試"]
    assert entries == [{"raw": "GBT 測試", "text": "GPT 測試", "duration_s": 1.0, "status": "pasted"}]
    assert main._sm.state is main.State.IDLE
