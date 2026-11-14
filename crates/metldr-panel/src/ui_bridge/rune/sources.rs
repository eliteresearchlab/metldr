use super::*;
use crate::ui_bridge::saved_source::{
    self, UiSourceSnapshot, UiSourceStatus, UiSourceSubscription,
};
use std::rc::Weak;

#[derive(Default)]
struct Inbox {
    snapshot: Option<UiSourceSnapshot>,
    refreshed: Option<Result<(), FlowError>>,
}

#[derive(Default)]
pub(super) struct Sources {
    owner: Weak<Core>,
    subscription: Option<UiSourceSubscription>,
    inbox: Rc<RefCell<Inbox>>,
    snapshot: Option<UiSourceSnapshot>,
    expanded: bool,
    captured_text: Option<String>,
    refreshing: bool,
    error: Option<String>,
}

impl Sources {
    pub(super) fn sync(&mut self, core: &Rc<Core>, updates: &Rc<Updates>) {
        if self.owner.as_ptr() != Rc::as_ptr(core) {
            *self = Self::default();
            self.owner = Rc::downgrade(core);
        }
        if self.subscription.is_none() {
            let inbox = self.inbox.clone();
            let updates = Rc::downgrade(updates);
            match saved_source::subscribe_rust(core, move |snapshot| {
                inbox.borrow_mut().snapshot = Some(snapshot);
                if let Some(updates) = updates.upgrade()
                    && !updates.closed.get()
                {
                    updates.changed();
                }
            }) {
                Ok(subscription) => self.subscription = Some(subscription),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        let snapshot = self.inbox.borrow_mut().snapshot.take();
        if let Some(snapshot) = snapshot {
            if self.snapshot.as_ref().and_then(UiSourceSnapshot::source_id) != snapshot.source_id()
            {
                self.captured_text = None;
            }
            self.snapshot = Some(snapshot);
            self.error = None;
            self.load_text();
        }
        if let Some(result) = self.inbox.borrow_mut().refreshed.take() {
            self.refreshing = false;
            if let Err(error) = result {
                self.error = Some(error.to_string());
            }
        }
        debug_assert!(self.expanded || self.captured_text.is_none());
    }

    fn load_text(&mut self) {
        if self.expanded
            && self.captured_text.is_none()
            && let Some(snapshot) = &self.snapshot
        {
            match snapshot.read_text() {
                Ok(text) => self.captured_text = text,
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    fn toggle(state: &mut State, _: &mut Tx) {
        state.sound(crate::interaction_audio::Cue::Tick);
        let source = &mut state.sources;
        source.expanded = !source.expanded;
        source.captured_text = None;
        source.load_text();
    }

    fn refresh(state: &mut State, _: &mut Tx) {
        let (Some(core), Some(updates)) = (state.core().cloned(), state.updates.clone()) else {
            return;
        };
        let source = &mut state.sources;
        if source.refreshing || source.owner.as_ptr() != Rc::as_ptr(&core) {
            return;
        }
        source.refreshing = true;
        source.error = None;
        let inbox = source.inbox.clone();
        let updates = Rc::downgrade(&updates);
        state.sound(crate::interaction_audio::Cue::Press);
        spawn_local(async move {
            let result = saved_source::refresh_rust(&core).await;
            // A replaced conversation owns a different inbox and never accepts this completion.
            inbox.borrow_mut().refreshed = Some(result);
            if let Some(updates) = updates.upgrade()
                && !updates.closed.get()
            {
                updates.changed();
            }
        });
    }

    pub(super) fn view(
        &self,
        typography: super::preferences::Typography,
        p: &super::preferences::Palette,
    ) -> El {
        let status = match self.snapshot.as_ref().map(UiSourceSnapshot::status) {
            None => "Loading saved page",
            Some(UiSourceStatus::Empty) => "No saved page",
            Some(UiSourceStatus::Checking) => "Checking page",
            Some(UiSourceStatus::Current) => "Matches current page",
            Some(UiSourceStatus::Changed) => "Page has changed",
            Some(UiSourceStatus::Saved) => "Saved capture",
            Some(UiSourceStatus::Unavailable) => "Page check unavailable",
        };
        let mut result = col().key("saved-source").w_full().gap(8.0).child(
            row()
                .w_full()
                .min_h(44.0)
                .gap(8.0)
                .items_center()
                .child(
                    row()
                        .key("saved-source-toggle")
                        .gap(8.0)
                        .min_h(44.0)
                        .items_center()
                        .child(
                            typography
                                .text("Saved page")
                                .font(typography.font(13.0, 500.0))
                                .color(p.ink),
                        )
                        .child(
                            ::rune::el::icon(&super::composer::CHEVRON, 11.0)
                                .color(p.ink_3)
                                .icon_rotation(if self.expanded { std::f32::consts::PI } else { 0.0 }),
                        )
                        .role("button")
                        .label(if self.expanded {
                            "Hide saved page"
                        } else {
                            "Show saved page"
                        })
                        .focusable(true)
                        .expanded(self.expanded)
                        .press_scale(super::theme::PRESS_SCALE)
                        .on_press_tx(Self::toggle)
                        .on_key(|state: &mut State, key, tx| {
                            composer::enter(state, key, tx, Self::toggle)
                        }),
                )
                .child(
                    typography
                        .text(status)
                        .font(typography.font(13.0, 400.0))
                        .color(p.ink_2)
                        .truncate()
                        .min_w(0.0)
                        .role("status"),
                ),
        );
        if !self.expanded {
            return result;
        }
        if let Some(snapshot) = &self.snapshot
            && snapshot.source_id().is_some()
        {
            let title = snapshot
                .title()
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "Untitled page".into());
            result = result
                .child(
                    typography
                        .text(title)
                        .font(typography.font(16.0, 600.0))
                        .role("heading")
                        .wrap(),
                )
                .child(
                    typography
                        .text(snapshot.url().unwrap_or_default())
                        .wrap()
                        .selectable(true),
                )
                .child(
                    col()
                        .key("saved-source-text")
                        .w_full()
                        .max_h(16.0 * 19.5 * typography.scale)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(
                            typography
                                .text(
                                    self.captured_text
                                        .as_deref()
                                        .unwrap_or("Loading captured text"),
                                )
                                .font(typography.font(13.0, 400.0).lh(19.5 * typography.scale))
                                .w_full()
                                .wrap()
                                .selectable(true),
                        ),
                );
        } else {
            result = result.child(
                typography
                    .text("Capture a page to use it in this conversation.")
                    .wrap(),
            );
        }
        if let Some(error) = &self.error {
            result = result.child(
                typography
                    .text(format!("The saved page could not be read. {error}"))
                    .role("alert")
                    .wrap(),
            );
        }
        let enabled = !self.refreshing && self.owner.upgrade().is_some();
        let mut check = typography
            .text(if self.refreshing {
                "Checking page"
            } else {
                "Check page"
            })
            .key("saved-source-refresh")
            .role("button")
            .min_h(44.0)
            .focusable(enabled)
            .disabled(!enabled)
            .padding(6.0);
        if enabled {
            check = check
                .press(typography.press)
                .press_scale(super::theme::PRESS_SCALE)
                .on_press_tx(Self::refresh)
                .on_key(|state: &mut State, key, tx| {
                    composer::enter(state, key, tx, Self::refresh)
                });
        }
        result.child(check)
    }
}
