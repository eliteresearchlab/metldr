//! Pointer press, drag, drag-out, cursor and wheel/fling input entry.
use super::*;

/// How far (points, per axis) a click may travel and still pair into a double click.
pub(super) const CLICK_SLOP: f32 = 4.0;

impl<S: 'static> Runtime<S> {
    pub fn pointer_move(&mut self, x: f32, y: f32) { self.move_pointer(x, y, true); }

    /// `drag_move` false keeps a held drag from getting a `Phase::Move`: a release at the
    /// point the drag last saw has nothing new to report, and its instant transaction would
    /// cut short whatever the press started (a tapped slider's knob mid-spring).
    pub(super) fn move_pointer(&mut self, x: f32, y: f32, drag_move: bool) {
        if self.core.pointer != Some((x, y)) { self.core.hover_paused = false; self.core.release_hover_hold(); }
        self.core.pointer = Some((x, y));
        if self.core.in_viewport(x, y) { self.core.left_in_capture = false; }
        for &(id, rect) in &self.core.hits {
            if self.core.hit_contains(id, rect, x, y)
                && let Some(&i) = self.core.node_index.get(&id)
                && self.core.nodes[i].handlers.get().is_some_and(|h| h.on_pointer_move.is_some())
            {
                let point = self.core.pointer_event(i, x, y);
                self.core.queue.push(Action::PointerMove(self.core.nodes[i].path.clone(), point));
            }
        }
        self.core.refresh_hover(x, y);
        let Some((path, d)) = self.core.capture.clone() else { return };
        if self.core.drag_out_armed && crate::drag_out::past_threshold(self.core.press_at, (x, y), self.core.drag_out_threshold) && self.start_drag_out(path.clone()) {
            return;
        }
        self.core.press_moved |= (x - self.core.press_at.0).abs() > CLICK_SLOP || (y - self.core.press_at.1).abs() > CLICK_SLOP;
        self.selection_drag(x, y);
        if self.core.input_click.captured {
            self.core.input_drag(x, y, drag_move);
            return;
        }
        let Some(i) = self.core.live_index(&path) else { return };
        let (id, r, has_drag) = (self.core.nodes[i].id, self.core.nodes[i].target, self.core.nodes[i].handlers.on_drag.is_some() && !self.core.drags_out(&self.core.nodes[i]));
        // A hit rect is where the element was drawn (after scrolling); `target` is its
        // unscrolled layout rect, kept only for elements that were never drawn.
        let mut hits = self.core.hits.iter().filter(|(hit, _)| *hit == id).peekable();
        self.core.capture_inside = !self.core.press_cancelled && if self.core.native_overlay || hits.peek().is_some() {
            hits.any(|&(id, r)| self.core.hit_contains(id, r, x, y))
        } else { contains(r, x, y) };
        if has_drag && drag_move {
            let (dx, dy) = (x - self.core.press_at.0, y - self.core.press_at.1);
            let [lx, ly] = self.core.to_local(i, x, y);
            let d = Drag { phase: Phase::Move, x: lx, y: ly, dx, dy, ..d };
            self.core.queue.push(Action::Drag(path, d, self.core.capture_button));
        }
    }

    /// Overlay observation updates coordinates without presenting unchanged hover pixels.
    pub(crate) fn observe_pointer(&mut self, x: f32, y: f32) -> bool {
        if self.core.pointer == Some((x, y)) { return false; }
        let visual_changed = self.core.nodes.iter().enumerate().any(|(i, n)| {
            if n.vis.hover.a <= 0.0 && n.vis.press.a <= 0.0 && n.handlers.get().is_none_or(|h| h.press_scale.is_none() && h.press_travel.is_none()) { return false; }
            let inside = |p: Option<(f32, f32)>| p.is_some_and(|(x, y)| self.core.node_contains(i, x, y));
            inside(self.core.pointer) != inside(Some((x, y)))
        });
        // Reaching or leaving a help element starts or ends its tooltip's wait.
        let help_changed = self.core.help_target_changes(x, y);
        let (queued, capture_inside) = (self.core.queue.len(), self.core.capture_inside);
        self.pointer_move(x, y);
        visual_changed || help_changed || self.core.queue.len() != queued || self.core.capture_inside != capture_inside
    }

    /// The held press crossed the drag threshold on a `drag_out` element: ask the host for an
    /// OS drag and hand it the pointer, so nothing stays pressed or hovered after the drop.
    /// False when the element offers nothing this time and the press stays a press.
    pub(super) fn start_drag_out(&mut self, path: String) -> bool {
        self.core.drag_out_armed = false;
        let Some(i) = self.core.live_index(&path) else { return false };
        let Some(f) = self.core.nodes[i].handlers.drag_out.clone() else { return false };
        let Some(item) = f(playground_host::handler_states_ref(&self.state, self.playground.as_deref())).filter(|item| !item.is_empty()) else { return false };
        let n = &self.core.nodes[i];
        let r = self.core.drawn.get(i).copied().unwrap_or([n.target[0], n.target[1], n.target[2] - n.target[0], n.target[3] - n.target[1]]);
        let clip = self.core.screen_clip_of_parent(n.parent).map(|v| v / self.core.env.scale);
        let (x0, y0) = (r[0].max(clip[0]), r[1].max(clip[1]));
        let rect = [x0, y0, ((r[0] + r[2]).min(clip[2]) - x0).max(0.0), ((r[1] + r[3]).min(clip[3]) - y0).max(0.0)];
        log_event!(self, "drag-out {path}");
        self.core.drop_drag_out_request();
        self.core.queue.push(Action::DragStart(path.clone()));
        self.core.drag_out_request = Some(crate::drag_out::DragOutStart { path, item, rect, bounds: r, press: self.core.press_at });
        self.core.capture = None;
        self.core.capture_inside = false;
        self.core.pointer_leave();
        true
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) {
        if self.core.capture.is_some() && self.core.capture_button != crate::PointerButton::Primary {
            self.core.pointer = Some((x, y));
            return;
        }
        self.core.drop_drag_out_request();
        self.core.pointer = Some((x, y));
        // A press is the hand's: hover follows the pointer through moving layout.
        self.core.release_hover_hold();
        self.core.keyboard_focus = false;
        self.core.dismiss_help();
        // A press cancels the waits to report an enter; the next move after it starts them again.
        self.core.hover_pending.retain(|p| !p.1);
        self.core.hover_paused = true;
        let target = self.core.primary_target(x, y);
        if target.and_then(|t| self.core.textbox_input(t)).is_none() {
            self.core.input_click.clear();
        }
        if let Some(l) = self.core.top_layer() {
            let reason = if !target.is_some_and(|t| self.core.inside_layer(l, t)) && !self.core.point_in_layer(l, x, y) {
                Some(crate::el::Dismiss::OutsidePress)
            } else {
                self.core.on_anchor(l, target, x, y).then_some(crate::el::Dismiss::AnchorPress)
            };
            if let Some(reason) = reason {
                self.core.queue.push(Action::LayerDismiss(self.core.nodes[l].path.clone(), reason));
            }
        }
        if let Some(i) = self.core.active_modal()
            && target.is_none_or(|t| !self.core.allowed(t))
        {
            if self.core.nodes[i].handlers.modal.as_ref().is_some_and(|m| m.outside) {
                self.core.queue.push(Action::Dismiss(self.core.nodes[i].path.clone()));
            }
            return;
        }
        self.playground_pointer_down(x);
        self.press_selection(target, x, y);
        // Inside a draggable row, dragging drags the row; clicks still select words and lines.
        if target.is_some_and(|t| self.core.nodes[t].handlers.on_drag.is_some() || self.core.nodes[t].handlers.drag_out.is_some()) {
            self.core.text_sel.dragging = false;
        }
        let Some(ti) = target else {
            let now = self.core.now_clock();
            self.core.titlebar.press_empty(now, x, y, self.core.double_click_interval);
            if self.core.focused.is_some() {
                self.core.queue.push(Action::Blur);
            }
            return;
        };
        let on_link = self.core.link_down(ti, x, y);
        // Any press breaks a pending pair unless it's the second press of it.
        let pending = self.core.last_press.take();
        let now = self.core.now_clock();
        let n = &self.core.nodes[ti];
        if n.window_drag && n.handlers.on_press.is_none() && n.handlers.on_double_press.is_none() && n.handlers.on_drag.is_none() && n.handlers.drag_out.is_none() && self.core.textbox_input(ti).is_none() {
            self.core.titlebar.press_region(now, x, y, self.core.double_click_interval);
            if self.core.focused.is_some() {
                self.core.queue.push(Action::Blur);
            }
            return;
        }
        self.core.press_at = (x, y);
        let clicks = self.core.press_clicks.press(&n.path, now, (x, y), self.core.double_click_interval);
        self.core.double_armed = n.handlers.on_double_press.is_some() && pending.is_some_and(|(p, t, px, py)| {
            p == n.path && now - t <= self.core.double_click_interval && (x - px).abs() <= CLICK_SLOP && (y - py).abs() <= CLICK_SLOP
        });
        self.core.press_down_t = now;
        self.core.press_moved = false;
        self.core.press_cancelled = false;
        if let Some(ji) = self.core.textbox_input(ti) {
            self.core.input_down(ji, x, y);
            return;
        }
        // Clicking a link leaves focus where it is (a composer keeps typing). Selectable text
        // off its links isn't a control: the press blurs, as a click on plain content does.
        // Inside a control that owns its keyboard (`El::on_key`), a click on any part of it
        // focuses it, as a click on a table view's row focuses the table.
        let keyed = (!n.focus.enabled && !self.core.is_selectable(ti)).then(|| std::iter::successors(n.parent, |&p| self.core.nodes[p].parent)
            .take_while(|&p| self.core.allowed(p))
            .find(|&p| self.core.nodes[p].focus.key.is_some() && self.core.can_focus(p))).flatten();
        let in_textbox = std::iter::successors(n.parent, |&p| self.core.nodes[p].parent).any(|p| self.core.textbox_input(p).is_some());
        let (preserve_focus, keeps) = (self.core.press_keeps_focus(ti), self.core.press_keeps_selection(ti));
        if !on_link && !keeps && n.focus.enabled && !self.core.is_selectable(ti) {
            self.core.queue.push(Action::FocusControl(n.path.clone()));
        } else if !on_link && !keeps && let Some(p) = keyed {
            self.core.queue.push(Action::FocusControl(self.core.nodes[p].path.clone()));
        } else if !on_link && !in_textbox && !preserve_focus && self.core.focused.is_some() {
            self.core.queue.push(Action::Blur);
        }
        let r = n.target;
        let [lx, ly] = self.core.to_local(ti, x, y);
        let d = Drag {
            phase: Phase::Down,
            x: lx,
            y: ly,
            w: r[2] - r[0],
            h: r[3] - r[1],
            start_x: lx,
            start_y: ly,
            dx: 0.0,
            dy: 0.0,
            data: 0.0,
            clicks,
            cancelled: false,
        };
        let path = n.path.clone();
        if n.handlers.on_press_down.is_some() && !n.a11y.disabled && !n.a11y.more.as_ref().is_some_and(|m| m.unavailable) {
            self.core.queue.push(Action::PressDown(path.clone()));
        }
        self.core.drag_out_armed = self.core.drags_out(n);
        self.core.drag_out_threshold = n.handlers.drag_threshold.unwrap_or(crate::drag_out::DRAG_THRESHOLD);
        if n.handlers.on_drag.is_some() && !self.core.drag_out_armed {
            self.core.queue.push(Action::Drag(path.clone(), d, crate::PointerButton::Primary));
        }
        self.core.capture = Some((path, d));
        self.core.capture_button = crate::PointerButton::Primary;
        self.core.capture_inside = true;
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) {
        if self.core.capture.is_some() && self.core.capture_button != crate::PointerButton::Primary {
            self.core.pointer = Some((x, y));
            return;
        }
        self.release_capture(x, y);
    }

    pub(super) fn release_capture(&mut self, x: f32, y: f32) {
        // Hosts release at the last position they saw, which a leave doesn't move.
        let left = self.core.left_in_capture;
        self.release(x, y);
        self.core.left_in_capture = left;
        self.core.leave_after_release();
    }

    pub(super) fn release(&mut self, x: f32, y: f32) {
        // A release never starts a drag-out: the system session needs the button held.
        self.core.drag_out_armed = false;
        let moved = self.core.pointer != Some((x, y));
        self.move_pointer(x, y, moved);
        // A drag that selected text is a selection, not a press.
        let selected = self.selected_by_drag();
        self.core.text_sel.dragging = false;
        let input_capture = self.core.input_click.end_press();
        self.core.end_press_repeat();
        let Some((path, d)) = self.core.capture.take() else { return };
        let button = std::mem::take(&mut self.core.capture_button);
        self.core.refresh_hover_within();
        let double = std::mem::take(&mut self.core.double_armed);
        if self.core.press_moved { self.core.press_clicks.clear(); }
        if input_capture {
            self.core.capture_inside = false;
            return;
        }
        let Some(i) = self.core.live_index(&path) else { return };
        let n = &self.core.nodes[i];
        let (has_drag, has_press, has_double) = (n.handlers.on_drag.is_some() && !self.core.drags_out(n), n.handlers.on_press.is_some(), n.handlers.on_double_press.is_some());
        if has_drag {
            let (dx, dy) = (x - self.core.press_at.0, y - self.core.press_at.1);
            let [lx, ly] = self.core.to_local(i, x, y);
            self.core.queue.push(Action::Drag(path.clone(), Drag { phase: Phase::Up, x: lx, y: ly, dx, dy, ..d }, button));
        }
        self.core.link_up(&path, x, y);
        if has_press && self.core.capture_inside && !selected {
            self.core.queue.push(Action::Press(path.clone(), button));
        }
        // Only clicks pair: a press that dragged or was held past the interval is neither half.
        let click = !self.core.press_moved && self.core.now_clock() - self.core.press_down_t <= self.core.double_click_interval;
        if has_double && self.core.capture_inside && click && button == crate::PointerButton::Primary {
            if double {
                self.core.queue.push(Action::DoublePress(path));
            } else {
                self.core.last_press = Some((path, self.core.press_down_t, self.core.press_at.0, self.core.press_at.1));
            }
        }
        self.core.capture_inside = false;
    }

    /// The pointer shape the shell should show: the pressed element's while a gesture holds
    /// the pointer, else the one under it in the current frame (the held hover while moving
    /// layout holds it), from the nearest ancestor that sets one. The arrow while the pointer is outside the window.
    pub fn cursor(&self) -> crate::el::Cursor {
        let Some((x, y)) = self.core.pointer.filter(|&(x, y)| self.core.in_viewport(x, y)) else { return crate::el::Cursor::Default };
        // An element on its way out has no cursor: `Cursor::Hidden` ends when it's removed.
        // Selectable text shows its I-beam even where its presses go to a row around it.
        let target = match &self.core.capture {
            Some(_) if self.core.text_sel.dragging => self.selection_holder(),
            Some((p, _)) => self.core.live_index(p).filter(|&i| self.core.allowed(i)),
            // Moving layout holds hover: the cursor stays with what the pointer was over.
            None => self.core.held_hover().flatten().or_else(|| self.core.selectable_at(x, y).or_else(|| self.core.primary_target(x, y))),
        };
        let Some(mut i) = target else {
            return crate::el::Cursor::Default;
        };
        if self.core.nodes[i].handlers.cursor.is_none() && (self.core.nodes[i].input.is_some() || self.core.nodes[i].role == "textbox") {
            return crate::el::Cursor::Text;
        }
        if self.core.is_selectable(i) && !self.core.text_sel.dragging && self.core.link_at(i, x, y).is_some() {
            return crate::el::Cursor::Pointer;
        }
        loop {
            if let Some(c) = self.core.nodes[i].handlers.cursor {
                return c;
            }
            let Some(parent) = self.core.nodes[i].parent else { return crate::el::Cursor::Default };
            i = parent;
        }
    }

    /// Wheel or trackpad delta in points (positive `dy` moves content down, like a browser's
    /// negative deltaY). Applied 1:1: the platform supplies its own momentum phases.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        self.core.release_hover_hold();
        self.core.drop_scroll_holds();
        self.core.dismiss_help();
        let Some(id) = self.core.scroll_at(x, y) else { return };
        let stick = self.core.node_index.get(&id).and_then(|&i| self.core.nodes[i].scroll).is_some_and(|s| s.stick_bottom);
        if let Some(st) = self.core.scrolls.get_mut(&id) {
            let before = st.offset;
            st.offset = (st.offset - f64::from(dy)).clamp(0.0, st.max);
            st.v = 0.0;
            st.glide = None;
            st.direct = true;
            st.pinned = stick && st.offset >= st.max - 0.5;
            log_event!(self.core, "wheel {dy:.1}");
            if st.offset != before { self.core.scroll_cancels_press(id); }
        }
    }
}

impl RuntimeCore {
    pub(crate) fn pointer_captured(&self) -> bool { self.capture.is_some() || self.secondary_capture.is_some() }

    /// The host moved its window by (dx, dy) points under a still pointer. Keep the pointer and
    /// a held press's origin in window points, so a drag that moves its own window still reports
    /// pointer travel in `Drag::dx`/`dy`.
    pub(crate) fn window_moved(&mut self, dx: f32, dy: f32) {
        if let Some((x, y)) = self.pointer { self.pointer = Some((x - dx, y - dy)); }
        self.press_at = (self.press_at.0 - dx, self.press_at.1 - dy);
    }

    pub(crate) fn cancel_pointer(&mut self) {
        self.cancel_drag();
        self.drop_drag_out_request();
        self.drag_out_armed = false;
        self.text_sel.dragging = false;
        self.input_click.clear();
        self.capture = None;
        self.secondary_capture = None;
        self.capture_inside = false;
        self.pointer_leave();
        self.drag_files_leave();
    }

    /// End a held drag without a drop: its handler hears `Phase::Up` with [`Drag::cancelled`]
    /// and the release that follows does nothing. False when no drag is held.
    pub(super) fn cancel_drag(&mut self) -> bool {
        if self.input_click.captured { return false; }
        let Some((path, d)) = &self.capture else { return false };
        let Some(i) = self.live_index(&path) else { return false };
        if self.nodes[i].handlers.on_drag.is_none() || self.drags_out(&self.nodes[i]) { return false; }
        let (x, y) = self.pointer.unwrap_or(self.press_at);
        let [lx, ly] = self.to_local(i, x, y);
        let (dx, dy) = (x - self.press_at.0, y - self.press_at.1);
        let (path, d) = (path.clone(), Drag { phase: Phase::Up, x: lx, y: ly, dx, dy, cancelled: true, ..*d });
        let button = std::mem::take(&mut self.capture_button);
        self.queue.push(Action::Drag(path, d, button));
        self.capture = None;
        self.end_press_repeat();
        self.capture_inside = false;
        self.double_armed = false;
        self.drag_out_armed = false;
        self.text_sel.dragging = false;
        true
    }

    /// A request no host took (headless tools, a cancelled press) ends as `Cancelled`, queued
    /// behind its start, so `on_drag_start` is always followed by `on_drag_end`.
    pub(super) fn drop_drag_out_request(&mut self) {
        if let Some(start) = self.drag_out_request.take() {
            self.queue.push(Action::DragEnd(start.path, crate::DragOutcome::Cancelled));
        }
    }

    /// A release outside the window, or after the pointer left during the press, ends the hover.
    pub(super) fn leave_after_release(&mut self) {
        if self.pointer_captured() { return; }
        if std::mem::take(&mut self.left_in_capture) || self.pointer.is_some_and(|(x, y)| !self.in_viewport(x, y)) {
            self.pointer_leave();
        }
    }

    pub fn pointer_leave(&mut self) {
        self.wheel_lost();
        if self.capture.is_none() && self.secondary_capture.is_none() {
            self.pointer = None;
            self.hovered = None;
            self.left_in_capture = false;
            self.refresh_hover_within();
        } else {
            self.left_in_capture = true;
        }
    }

    /// A press on an element is held (the hand drives whatever follows it).
    pub fn pressing(&self) -> bool {
        self.capture.is_some()
    }
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    /// Native overlay regions from the most recent frame, in logical window points (x0,y0,x1,y1).
    pub fn hit_regions(&self) -> &[[f32; 4]] { &self.overlay_hits }

    /// The innermost vertical scroll viewport at (x, y).
    pub(super) fn scroll_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.scroll_at_axis(x, y, false)
    }

    /// The innermost scroll viewport at (x, y) that moves on the horizontal axis or the vertical.
    pub(super) fn scroll_at_axis(&self, x: f32, y: f32, horizontal: bool) -> Option<NodeId> {
        self.nodes
            .iter()
            .enumerate()
            .rev()
            .filter(|(i, n)| n.scroll.is_some() && n.scroll_axes.has(horizontal) && !self.subtree_exiting(*i) && self.allowed(*i))
            .find(|(i, _)| self.node_contains(*i, x, y))
            .map(|(_, n)| n.id)
    }

    /// Releases a scroll with `velocity` (points per second, positive moves content down);
    /// the runtime decays it (for platforms without OS momentum and headless scripts).
    pub fn fling(&mut self, x: f32, y: f32, velocity: f32) {
        self.release_hover_hold();
        self.drop_scroll_holds();
        self.dismiss_help();
        let Some(id) = self.scroll_at(x, y) else { return };
        if let Some(st) = self.scrolls.get_mut(&id) {
            st.v = -velocity;
            st.pinned = false;
            st.glide = None;
            log_event!(self, "fling {velocity:.0}");
            if velocity != 0.0 { self.scroll_cancels_press(id); }
        }
    }

    pub fn has_pending_input(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Scrolling viewport `id` under a held press inside it cancels the press, as a UIScrollView
    /// cancels its content's touches: the row lets go and the release doesn't press. A drag
    /// keeps its capture.
    pub(super) fn scroll_cancels_press(&mut self, id: NodeId) {
        // A cancelled press stays cancelled until the next press, so later scroll steps skip the search.
        if self.press_cancelled { return; }
        let Some((path, _)) = &self.capture else { return };
        let Some(i) = self.nodes.iter().position(|n| &n.path == path) else { return };
        let inside = std::iter::successors(self.nodes[i].parent, |&p| self.nodes[p].parent).any(|p| self.nodes[p].id == id);
        if inside && self.nodes[i].handlers.on_drag.is_none() {
            self.press_cancelled = true;
            self.capture_inside = false;
            self.double_armed = false;
        }
    }
}
