//! The runtime's side of `perf_warn`'s commit watch: why each of a frame's commits ran, for the
//! frame log and `RUNE_COMMIT_BUDGET_MS`, and what the commit warnings hear once a frame.
use super::*;

impl<S: 'static> Runtime<S> {
    /// A frame's commit for `why` (`CommitWhy` bits).
    pub(super) fn commit_for(&mut self, tx: &Tx, why: u16) {
        self.core.commit_why |= why;
        if crate::tooling::interaction::on() { self.core.watch.interaction.committed(why); }
        self.core.watch.cause = why;
        self.commit(tx);
        self.core.watch.cause = 0;
    }
}

impl RuntimeCore {
    /// After a frame's commits, before its scroll requests run: whether it committed since the
    /// `commits0`th commit, while a viewport scrolled; `instant` is the viewport an instant scroll
    /// it asked for moves.
    #[cfg(debug_assertions)]
    pub(super) fn watch_frame(&mut self, commits0: u64, instant: Option<&str>) {
        let scrolling = self.scrolling();
        self.watch.meter.frame_ends(scrolling, self.commits != commits0, instant);
    }
}
