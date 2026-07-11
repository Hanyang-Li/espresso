#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SetSleepDisabled(bool),
    StartLidWatch,
    StopLidWatch,
    StartGraceTimer,
    CancelGraceTimer,
    Exit,
}

#[derive(Debug, Default)]
pub struct RefcountState {
    count: u32,
    in_grace: bool,
}

impl RefcountState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    /// Arms the idle-exit clock for a freshly-started daemon that begins at
    /// refcount 0 (e.g. socket-activated by a `Query` connection or
    /// relaunched by launchd after a crash, without ever receiving a
    /// `Hold`). Only meaningful when called immediately after `new()`, i.e.
    /// at count 0: it puts the state into the same "in grace" condition
    /// `on_hold_close` reaches when the last hold releases, so the daemon
    /// still self-exits after the grace period if no `Hold` ever arrives.
    pub fn begin_idle(&mut self) -> Vec<Action> {
        self.in_grace = true;
        vec![Action::StartGraceTimer]
    }

    pub fn on_hold_open(&mut self) -> Vec<Action> {
        self.count += 1;
        if self.count != 1 {
            return vec![];
        }
        let mut actions = Vec::new();
        if self.in_grace {
            self.in_grace = false;
            actions.push(Action::CancelGraceTimer);
        }
        actions.push(Action::SetSleepDisabled(true));
        actions.push(Action::StartLidWatch);
        actions
    }

    pub fn on_hold_close(&mut self) -> Vec<Action> {
        if self.count == 0 {
            return vec![];
        }
        self.count -= 1;
        if self.count != 0 {
            return vec![];
        }
        self.in_grace = true;
        vec![
            Action::SetSleepDisabled(false),
            Action::StopLidWatch,
            Action::StartGraceTimer,
        ]
    }

    pub fn on_grace_elapsed(&mut self) -> Vec<Action> {
        if self.count == 0 && self.in_grace {
            self.in_grace = false;
            vec![Action::Exit]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::*;

    #[test]
    fn first_hold_enables_and_watches() {
        let mut s = RefcountState::new();
        assert_eq!(s.on_hold_open(), vec![SetSleepDisabled(true), StartLidWatch]);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn second_hold_is_noop() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        assert_eq!(s.on_hold_open(), vec![]);
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn last_close_disables_and_starts_grace() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        assert_eq!(
            s.on_hold_close(),
            vec![SetSleepDisabled(false), StopLidWatch, StartGraceTimer]
        );
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn non_last_close_is_noop() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_open();
        assert_eq!(s.on_hold_close(), vec![]);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn grace_elapsed_at_zero_exits() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_close();
        assert_eq!(s.on_grace_elapsed(), vec![Exit]);
    }

    #[test]
    fn hold_during_grace_cancels_and_reenables() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_close(); // enters grace
        assert_eq!(
            s.on_hold_open(),
            vec![CancelGraceTimer, SetSleepDisabled(true), StartLidWatch]
        );
        // A stale grace-elapsed after re-enable must be ignored.
        assert_eq!(s.on_grace_elapsed(), vec![]);
    }

    #[test]
    fn over_close_from_zero_is_noop() {
        let mut s = RefcountState::new();
        assert_eq!(s.on_hold_close(), vec![]);
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn double_close_after_last_is_noop() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        assert_eq!(
            s.on_hold_close(),
            vec![SetSleepDisabled(false), StopLidWatch, StartGraceTimer]
        );
        assert_eq!(s.on_hold_close(), vec![]);
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn grace_elapsed_then_reopen_is_clean() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_close(); // enters grace
        assert_eq!(s.on_grace_elapsed(), vec![Exit]);
        assert_eq!(s.on_grace_elapsed(), vec![]);
    }

    #[test]
    fn begin_idle_then_grace_exits() {
        let mut s = RefcountState::new();
        assert_eq!(s.begin_idle(), vec![StartGraceTimer]);
        assert_eq!(s.count(), 0);
        assert_eq!(s.on_grace_elapsed(), vec![Exit]);
    }

    #[test]
    fn begin_idle_then_hold_cancels() {
        let mut s = RefcountState::new();
        s.begin_idle();
        assert_eq!(
            s.on_hold_open(),
            vec![CancelGraceTimer, SetSleepDisabled(true), StartLidWatch]
        );
        // A stale grace-elapsed from the startup timer must be ignored.
        assert_eq!(s.on_grace_elapsed(), vec![]);
    }
}
