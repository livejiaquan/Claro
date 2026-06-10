from enum import Enum, auto

TAP_THRESHOLD_S = 0.35


class State(Enum):
    IDLE = auto()
    HOLD = auto()        # 錄音中，熱鍵按住
    HANDSFREE = auto()   # 錄音中，免持
    PROCESSING = auto()  # 轉錄/潤飾中


class Action(Enum):
    NONE = auto()
    START_RECORDING = auto()
    ENTER_HANDSFREE = auto()
    STOP_AND_PROCESS = auto()
    CANCEL_RECORDING = auto()
    CANCEL_PROCESSING = auto()


class DictationStateMachine:
    """純邏輯狀態機：吃鍵盤事件、吐動作。不碰音訊/UI，方便測試。"""

    def __init__(self):
        self.state = State.IDLE
        self._down_ts = 0.0
        self.session = 0

    def hotkey_down(self, ts: float) -> Action:
        if self.state is State.IDLE:
            self.session += 1
            self.state = State.HOLD
            self._down_ts = ts
            return Action.START_RECORDING
        if self.state is State.HANDSFREE:
            self.state = State.PROCESSING
            return Action.STOP_AND_PROCESS
        return Action.NONE

    def hotkey_up(self, ts: float) -> Action:
        if self.state is State.HOLD:
            if ts - self._down_ts < TAP_THRESHOLD_S:
                self.state = State.HANDSFREE
                return Action.ENTER_HANDSFREE
            self.state = State.PROCESSING
            return Action.STOP_AND_PROCESS
        return Action.NONE

    def esc(self) -> Action:
        if self.state in (State.HOLD, State.HANDSFREE):
            self.state = State.IDLE
            return Action.CANCEL_RECORDING
        if self.state is State.PROCESSING:
            self.state = State.IDLE
            return Action.CANCEL_PROCESSING
        return Action.NONE

    def force_stop(self) -> Action:
        if self.state in (State.HOLD, State.HANDSFREE):
            self.state = State.PROCESSING
            return Action.STOP_AND_PROCESS
        return Action.NONE

    def processing_finished(self, session: int):
        if self.state is State.PROCESSING and self.session == session:
            self.state = State.IDLE
