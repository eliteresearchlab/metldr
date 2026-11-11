//! Phases of the internal Rune host. Plain state: no Rune, DOM or wasm types, so the native tests
//! cover duplicate mounts, stale callbacks and asynchronous closing without a renderer.

/// Where the host is in one mount's life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Idle,
    /// Requested. The renderer has not reported a live surface yet.
    Starting,
    /// The renderer owns a live surface.
    Running,
    /// Quit requested. The renderer has not been torn down yet, so the document stays reserved.
    Closing,
}

/// One mount attempt. Callbacks carry it, so a callback from an earlier attempt cannot act on a
/// newer one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Session(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A mount is requested, running or still closing. Carries the phase that refused it.
    Busy(Phase),
    /// The callback belongs to an attempt that is no longer current, or the phase does not
    /// accept it.
    Stale,
}

#[derive(Debug, Default)]
pub(super) struct Lifecycle {
    phase: Phase,
    current: u64,
    issued: u64,
}

impl Lifecycle {
    pub(super) fn phase(&self) -> Phase {
        self.phase
    }

    /// The attempt most recently begun. Meaningful only while a mount is not idle.
    pub(super) fn current(&self) -> Session {
        Session(self.current)
    }

    /// Reserves the document for a new mount. Refused while another mount is live or closing.
    pub(super) fn begin(&mut self) -> Result<Session, Refusal> {
        if self.phase != Phase::Idle {
            return Err(Refusal::Busy(self.phase));
        }
        self.issued += 1;
        self.current = self.issued;
        self.phase = Phase::Starting;
        Ok(Session(self.current))
    }

    /// The renderer reported a live surface for `session`.
    pub(super) fn mounted(&mut self, session: Session) -> Result<(), Refusal> {
        self.check(session, Phase::Starting)?;
        self.phase = Phase::Running;
        Ok(())
    }

    /// Startup failed before a live surface existed. The document is free again.
    pub(super) fn failed(&mut self, session: Session) -> Result<(), Refusal> {
        self.check(session, Phase::Starting)?;
        self.phase = Phase::Idle;
        Ok(())
    }

    /// Begins closing a mount that is starting or running. Returns the session to quit, or
    /// `None` when there is nothing to close.
    pub(super) fn close(&mut self) -> Option<Session> {
        match self.phase {
            Phase::Starting | Phase::Running => {
                self.phase = Phase::Closing;
                Some(Session(self.current))
            }
            Phase::Idle | Phase::Closing => None,
        }
    }

    /// The renderer has been torn down for `session`. The document is free again.
    pub(super) fn unmounted(&mut self, session: Session) -> Result<(), Refusal> {
        self.check(session, Phase::Closing)?;
        self.phase = Phase::Idle;
        Ok(())
    }

    /// Whether `session` is the live attempt and its renderer may draw a frame.
    pub(super) fn accepts_frames(&self, session: Session) -> bool {
        session.0 == self.current && matches!(self.phase, Phase::Starting | Phase::Running)
    }

    fn check(&self, session: Session, expected: Phase) -> Result<(), Refusal> {
        if session.0 == self.current && self.phase == expected {
            Ok(())
        } else {
            Err(Refusal::Stale)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_mount_is_refused_while_one_is_starting_or_running() {
        let mut lifecycle = Lifecycle::default();
        let first = lifecycle.begin().expect("first mount begins");
        assert_eq!(lifecycle.begin(), Err(Refusal::Busy(Phase::Starting)));
        lifecycle
            .mounted(first)
            .expect("first mount reports a surface");
        assert_eq!(lifecycle.begin(), Err(Refusal::Busy(Phase::Running)));
    }

    #[test]
    fn a_closing_mount_holds_the_document_until_it_is_torn_down() {
        let mut lifecycle = Lifecycle::default();
        let first = lifecycle.begin().expect("first mount begins");
        lifecycle
            .mounted(first)
            .expect("first mount reports a surface");
        assert_eq!(lifecycle.close(), Some(first));
        assert_eq!(lifecycle.begin(), Err(Refusal::Busy(Phase::Closing)));
        lifecycle.unmounted(first).expect("teardown completes");
        assert_eq!(lifecycle.phase(), Phase::Idle);
        assert!(lifecycle.begin().is_ok());
    }

    #[test]
    fn startup_failure_frees_the_document_without_a_renderer() {
        let mut lifecycle = Lifecycle::default();
        let first = lifecycle.begin().expect("first mount begins");
        lifecycle
            .failed(first)
            .expect("startup failure is recorded");
        assert_eq!(lifecycle.phase(), Phase::Idle);
        assert!(!lifecycle.accepts_frames(first));
        assert!(lifecycle.begin().is_ok());
    }

    #[test]
    fn callbacks_from_an_earlier_mount_are_stale_after_a_restart() {
        let mut lifecycle = Lifecycle::default();
        let first = lifecycle.begin().expect("first mount begins");
        lifecycle
            .mounted(first)
            .expect("first mount reports a surface");
        lifecycle.close();
        lifecycle.unmounted(first).expect("teardown completes");

        let second = lifecycle.begin().expect("second mount begins");
        assert_eq!(lifecycle.mounted(first), Err(Refusal::Stale));
        assert_eq!(lifecycle.failed(first), Err(Refusal::Stale));
        assert_eq!(lifecycle.unmounted(first), Err(Refusal::Stale));
        assert!(!lifecycle.accepts_frames(first));
        assert!(lifecycle.accepts_frames(second));
        assert_eq!(lifecycle.mounted(second), Ok(()));
    }

    #[test]
    fn a_surface_reported_after_close_is_ignored_until_teardown() {
        let mut lifecycle = Lifecycle::default();
        let first = lifecycle.begin().expect("first mount begins");
        assert_eq!(lifecycle.close(), Some(first));
        assert_eq!(lifecycle.mounted(first), Err(Refusal::Stale));
        assert!(!lifecycle.accepts_frames(first));
        assert_eq!(lifecycle.phase(), Phase::Closing);
        lifecycle.unmounted(first).expect("teardown completes");
        assert_eq!(lifecycle.phase(), Phase::Idle);
    }

    #[test]
    fn close_is_a_no_op_when_nothing_is_live() {
        let mut lifecycle = Lifecycle::default();
        assert_eq!(lifecycle.close(), None);
        let first = lifecycle.begin().expect("first mount begins");
        assert_eq!(lifecycle.close(), Some(first));
        assert_eq!(lifecycle.close(), None);
    }
}
