from state_machine import Action, DictationStateMachine, State, TAP_THRESHOLD_S


def make():
    return DictationStateMachine()


def test_idle_keydown_starts_recording():
    sm = make()
    assert sm.hotkey_down(10.0) is Action.START_RECORDING
    assert sm.state is State.HOLD


def test_long_hold_release_stops_and_processes():
    sm = make()
    sm.hotkey_down(10.0)
    assert sm.hotkey_up(10.0 + TAP_THRESHOLD_S + 0.1) is Action.STOP_AND_PROCESS
    assert sm.state is State.PROCESSING


def test_quick_tap_enters_handsfree():
    sm = make()
    sm.hotkey_down(10.0)
    assert sm.hotkey_up(10.1) is Action.ENTER_HANDSFREE
    assert sm.state is State.HANDSFREE


def test_handsfree_second_press_stops():
    sm = make()
    sm.hotkey_down(10.0)
    sm.hotkey_up(10.1)
    assert sm.hotkey_down(15.0) is Action.STOP_AND_PROCESS
    assert sm.state is State.PROCESSING
    # 第二次按鍵的 keyUp 不應該有作用
    assert sm.hotkey_up(15.2) is Action.NONE
    assert sm.state is State.PROCESSING


def test_esc_cancels_recording_hold_and_handsfree():
    sm = make()
    sm.hotkey_down(10.0)
    assert sm.esc() is Action.CANCEL_RECORDING
    assert sm.state is State.IDLE

    sm.hotkey_down(20.0)
    sm.hotkey_up(20.1)
    assert sm.esc() is Action.CANCEL_RECORDING
    assert sm.state is State.IDLE


def test_esc_during_processing_cancels_processing():
    sm = make()
    sm.hotkey_down(10.0)
    sm.hotkey_up(11.0)
    assert sm.esc() is Action.CANCEL_PROCESSING
    assert sm.state is State.IDLE


def test_processing_finished_returns_to_idle():
    sm = make()
    sm.hotkey_down(10.0)
    sm.hotkey_up(11.0)
    sm.processing_finished()
    assert sm.state is State.IDLE


def test_ignored_events():
    sm = make()
    assert sm.hotkey_up(10.0) is Action.NONE      # idle 收到 keyUp
    assert sm.esc() is Action.NONE                 # idle 收到 esc
    sm.hotkey_down(10.0)
    assert sm.hotkey_down(10.2) is Action.NONE     # HOLD 中重複 keyDown
