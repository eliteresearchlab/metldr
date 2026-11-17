//! The runtime's side of `perf_warn`'s idle meters: what woke a frame, and what it drew.
use super::*;

impl<S: 'static> Runtime<S> {
    /// After a frame built its instances.
    #[inline(always)]
    pub(super) fn idle_frame_ends(&mut self) {
        #[cfg(debug_assertions)]
        {
            use std::hash::{Hash, Hasher};
            let mut key = std::collections::hash_map::DefaultHasher::new();
            (self.clear_color().map(f32::to_bits), self.core.viewport.0.to_bits(), self.core.viewport.1.to_bits(), self.core.env.scale.to_bits()).hash(&mut key);
            (self.core.srgb_blend, self.core.transparent, self.core.images.copies).hash(&mut key);
            let drawn = crate::tooling::perf_warn::Drawn {
                instances: bytemuck::cast_slice(&self.core.instances),
                key: key.finish(),
                opaque: !self.core.surfaces.is_empty() || !self.core.materials.is_empty() || self.core.videos_active,
            };
            let title = &self.core.window_title;
            self.core.idle_meter.ends(self.core.clock, self.core.animating, drawn, || title.current().map(str::to_owned));
        }
    }
}

impl RuntimeCore {
    /// Before a frame's poll. `timer`: a due `wake_after` woke it, or a resting wake's poll
    /// stands for it; `live`: a live value did.
    #[inline(always)]
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub(super) fn idle_frame_begins(&mut self, timer: bool, live: bool) {
        #[cfg(debug_assertions)]
        self.idle_meter.begins(self.queue.is_empty() && !self.animating, timer, live);
    }
}
