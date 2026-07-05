//! 純邏輯狀態機：吃鍵盤事件、吐動作。不碰音訊/UI，方便測試。
//! 自 prototype/state_machine.py 逐行移植，語意不得偏離；
//! 測試與 prototype/tests/test_state_machine.py 逐案對應。

/// 快點（tap）與長按（hold）的分界秒數
pub const TAP_THRESHOLD_S: f64 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    /// 錄音中，熱鍵按住
    Hold,
    /// 錄音中，免持
    Handsfree,
    /// 轉錄/潤飾中
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmAction {
    None,
    StartRecording,
    EnterHandsfree,
    StopAndProcess,
    CancelRecording,
    CancelProcessing,
}

pub struct DictationStateMachine {
    state: State,
    down_ts: f64,
    session: u64,
}

impl Default for DictationStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl DictationStateMachine {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            down_ts: 0.0,
            session: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    pub fn hotkey_down(&mut self, ts: f64) -> SmAction {
        match self.state {
            State::Idle => {
                self.session += 1;
                self.state = State::Hold;
                self.down_ts = ts;
                SmAction::StartRecording
            }
            State::Handsfree => {
                self.state = State::Processing;
                SmAction::StopAndProcess
            }
            _ => SmAction::None,
        }
    }

    pub fn hotkey_up(&mut self, ts: f64) -> SmAction {
        if self.state != State::Hold {
            return SmAction::None;
        }
        if ts - self.down_ts < TAP_THRESHOLD_S {
            self.state = State::Handsfree;
            SmAction::EnterHandsfree
        } else {
            self.state = State::Processing;
            SmAction::StopAndProcess
        }
    }

    pub fn esc(&mut self) -> SmAction {
        match self.state {
            State::Hold | State::Handsfree => {
                self.state = State::Idle;
                SmAction::CancelRecording
            }
            State::Processing => {
                self.state = State::Idle;
                SmAction::CancelProcessing
            }
            State::Idle => SmAction::None,
        }
    }

    pub fn force_stop(&mut self) -> SmAction {
        match self.state {
            State::Hold | State::Handsfree => {
                self.state = State::Processing;
                SmAction::StopAndProcess
            }
            _ => SmAction::None,
        }
    }

    pub fn processing_finished(&mut self, session: u64) {
        if self.state == State::Processing && self.session == session {
            self.state = State::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> DictationStateMachine {
        DictationStateMachine::new()
    }

    #[test]
    fn idle_keydown_starts_recording() {
        let mut sm = make();
        assert_eq!(sm.hotkey_down(10.0), SmAction::StartRecording);
        assert_eq!(sm.state(), State::Hold);
    }

    #[test]
    fn long_hold_release_stops_and_processes() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        assert_eq!(sm.hotkey_up(10.0 + TAP_THRESHOLD_S + 0.1), SmAction::StopAndProcess);
        assert_eq!(sm.state(), State::Processing);
    }

    #[test]
    fn quick_tap_enters_handsfree() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        assert_eq!(sm.hotkey_up(10.1), SmAction::EnterHandsfree);
        assert_eq!(sm.state(), State::Handsfree);
    }

    #[test]
    fn handsfree_second_press_stops() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        sm.hotkey_up(10.1);
        assert_eq!(sm.hotkey_down(15.0), SmAction::StopAndProcess);
        assert_eq!(sm.state(), State::Processing);
        // 第二次按鍵的 keyUp 不應該有作用
        assert_eq!(sm.hotkey_up(15.2), SmAction::None);
        assert_eq!(sm.state(), State::Processing);
    }

    #[test]
    fn esc_cancels_recording_hold_and_handsfree() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        assert_eq!(sm.esc(), SmAction::CancelRecording);
        assert_eq!(sm.state(), State::Idle);

        sm.hotkey_down(20.0);
        sm.hotkey_up(20.1);
        assert_eq!(sm.esc(), SmAction::CancelRecording);
        assert_eq!(sm.state(), State::Idle);
    }

    #[test]
    fn esc_during_processing_cancels_processing() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        sm.hotkey_up(11.0);
        assert_eq!(sm.esc(), SmAction::CancelProcessing);
        assert_eq!(sm.state(), State::Idle);
    }

    #[test]
    fn processing_finished_returns_to_idle() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        sm.hotkey_up(11.0);
        let session = sm.session();
        sm.processing_finished(session);
        assert_eq!(sm.state(), State::Idle);
    }

    #[test]
    fn processing_finished_ignores_stale_session_id() {
        let mut sm = make();
        sm.hotkey_down(10.0);
        let old_session = sm.session();
        sm.hotkey_up(11.0);
        assert_eq!(sm.esc(), SmAction::CancelProcessing);

        sm.hotkey_down(20.0);
        let current_session = sm.session();
        sm.hotkey_up(21.0);
        assert_eq!(sm.state(), State::Processing);
        sm.processing_finished(old_session);
        assert_eq!(sm.state(), State::Processing);
        sm.processing_finished(current_session);
        assert_eq!(sm.state(), State::Idle);
    }

    #[test]
    fn force_stop_moves_recording_to_processing_only() {
        let mut sm = make();
        assert_eq!(sm.force_stop(), SmAction::None);
        assert_eq!(sm.state(), State::Idle);

        sm.hotkey_down(10.0);
        assert_eq!(sm.force_stop(), SmAction::StopAndProcess);
        assert_eq!(sm.state(), State::Processing);

        let mut sm = make();
        sm.hotkey_down(20.0);
        sm.hotkey_up(20.1);
        assert_eq!(sm.force_stop(), SmAction::StopAndProcess);
        assert_eq!(sm.state(), State::Processing);
    }

    #[test]
    fn ignored_events() {
        let mut sm = make();
        assert_eq!(sm.hotkey_up(10.0), SmAction::None); // idle 收到 keyUp
        assert_eq!(sm.esc(), SmAction::None); // idle 收到 esc
        sm.hotkey_down(10.0);
        assert_eq!(sm.hotkey_down(10.2), SmAction::None); // HOLD 中重複 keyDown
    }
}
