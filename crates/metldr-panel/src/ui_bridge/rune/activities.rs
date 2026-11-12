//! Bounded activity reads and Rune presentation for the conversation.
use super::*;
use crate::ag_ui::{Activity, AgUiMessage};
use metldr_conversation::{Chart, ChartKind, Table};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

#[derive(Default)]
pub(super) struct Activities {
    inner: Rc<RefCell<Cache>>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    reading: bool,
    core: Weak<Core>,
    generation: u64,
    next_token: u64,
}

struct Entry {
    activity: Activity,
    value: Value,
    offset: u32,
    rect: ::rune::Live<[f32; 4]>,
    height: f32,
    token: u64,
    refresh_needed: bool,
    refreshing: bool,
    pump_allowed: Rc<Cell<bool>>,
}

enum Value {
    Pending,
    Loading,
    Waiting(Rc<UiAnswerFrame>),
    Failed,
    Table(Table),
    Chart(Chart),
    Generated(Rc<UiAnswerFrame>),
    Tools(UiNativeTools),
    Details(UiRunDetails),
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.pump_allowed.set(false);
        if let Value::Generated(frame) | Value::Waiting(frame) = &self.value {
            frame.close();
        }
    }
}

impl Activities {
    /// Call from poll after accepting conversation changes; one read runs at a time.
    pub(super) fn sync(&self, core: &Rc<Core>, messages: &[AgUiMessage], updates: &Rc<Updates>) {
        let mut cache = self.inner.borrow_mut();
        if cache.core.as_ptr() != Rc::as_ptr(core) {
            cache.entries.clear();
            cache.generation = cache.generation.wrapping_add(1);
        }
        cache.core = Rc::downgrade(core);
        cache.entries.retain(|id, _| {
            messages.iter().any(|message| {
                matches!(message,
            AgUiMessage::Activity { id: current, .. } if current == id)
            })
        });
        for message in messages {
            if let AgUiMessage::Activity { id, activity } = message {
                if let Some(entry) = cache.entries.get_mut(id) {
                    if entry.activity == *activity {
                        continue;
                    }
                    if let (Activity::Generated(old), Activity::Generated(new)) =
                        (&entry.activity, activity)
                        && old.run_id == new.run_id
                        && old.artifact_id == new.artifact_id
                    {
                        if new.revision.0 > old.revision.0 {
                            entry.activity = activity.clone();
                            entry.refresh_needed = true;
                            entry.pump_allowed.set(false);
                        }
                        continue;
                    }
                }
                cache.next_token = cache.next_token.wrapping_add(1);
                let token = cache.next_token;
                cache.entries.insert(
                    id.clone(),
                    Entry {
                        activity: activity.clone(),
                        value: Value::Pending,
                        offset: 0,
                        rect: ::rune::Live::new([0.0; 4]),
                        height: 200.0,
                        token,
                        refresh_needed: false,
                        refreshing: false,
                        pump_allowed: Rc::new(Cell::new(false)),
                    },
                );
            }
        }
        if cache.reading {
            return;
        }
        let Some((id, entry)) = cache.entries.iter_mut().find(|(_, entry)| {
            matches!(entry.value, Value::Pending)
                || (entry.refresh_needed
                    && matches!(entry.value, Value::Generated(_) | Value::Waiting(_)))
        }) else {
            return;
        };
        let (id, activity, token) = (id.clone(), entry.activity.clone(), entry.token);
        let frame = match &entry.value {
            Value::Generated(frame) | Value::Waiting(frame) => Some(Rc::clone(frame)),
            _ => None,
        };
        if frame.is_none() {
            entry.value = Value::Loading;
        }
        entry.refresh_needed = false;
        entry.refreshing = true;
        cache.reading = true;
        let generation = cache.generation;
        drop(cache);
        let (cache, core, updates) = (
            Rc::downgrade(&self.inner),
            Rc::clone(core),
            Rc::downgrade(updates),
        );
        spawn_local(async move {
            let result = read(core, &activity, frame.clone()).await;
            if let Some(cache) = cache.upgrade() {
                let mut cache = cache.borrow_mut();
                cache.reading = false;
                if cache.generation == generation
                    && let Some(entry) = cache.entries.get_mut(&id)
                    && entry.token == token
                    && entry.refreshing
                    && match (&frame, &entry.value) {
                        (None, Value::Loading) => true,
                        (Some(old), Value::Generated(current) | Value::Waiting(current)) => {
                            Rc::ptr_eq(old, current)
                        }
                        _ => false,
                    }
                {
                    entry.refreshing = false;
                    if entry.activity != activity {
                        entry.refresh_needed = true;
                        if frame.is_none() {
                            entry.value = match result {
                                Ok(Value::Generated(frame) | Value::Waiting(frame)) => {
                                    Value::Waiting(frame)
                                }
                                _ => Value::Pending,
                            };
                        }
                    } else {
                        let first_display =
                            matches!(&entry.value, Value::Loading | Value::Waiting(_));
                        let value = result.unwrap_or_else(|_| {
                            if let Some(frame) = &frame {
                                frame.close();
                            }
                            Value::Failed
                        });
                        let value = match (&entry.value, value) {
                            (Value::Generated(_), Value::Waiting(frame)) => {
                                entry.pump_allowed.set(false);
                                Value::Generated(frame)
                            }
                            (_, value) => {
                                entry
                                    .pump_allowed
                                    .set(matches!(&value, Value::Generated(_)));
                                value
                            }
                        };
                        if first_display && let Value::Generated(frame) = &value {
                            entry.height = frame.height() as f32;
                        }
                        entry.value = value;
                    }
                }
            }
            if let Some(updates) = updates.upgrade() {
                updates.changed();
            }
        });
    }

    /// The host must mount this controller in its restricted answer iframe.
    pub(super) fn answer_frame(&self, id: &str) -> Option<Rc<UiAnswerFrame>> {
        let cache = self.inner.borrow();
        let entry = cache.entries.get(id)?;
        match &entry.value {
            Value::Generated(frame)
                if matches!(&entry.activity, Activity::Generated(reference)
                    if reference.completion == metldr_conversation::ArtifactCompletion::Interrupted)
                    && frame.completion() != "interrupted" =>
            {
                None
            }
            Value::Generated(frame) => Some(Rc::clone(frame)),
            _ => None,
        }
    }

    pub(super) fn answer_rect(&self, id: &str) -> Option<::rune::Live<[f32; 4]>> {
        Some(self.inner.borrow().entries.get(id)?.rect.clone())
    }

    pub(super) fn answer_gate(&self, id: &str) -> Option<Rc<Cell<bool>>> {
        Some(Rc::clone(
            &self.inner.borrow().entries.get(id)?.pump_allowed,
        ))
    }

    pub(super) fn answer_height(&self, id: &str, updates: &Rc<Updates>) -> Rc<dyn Fn(f32)> {
        let cache = Rc::downgrade(&self.inner);
        let updates = Rc::downgrade(updates);
        let frame = self.answer_frame(id).map(|frame| Rc::downgrade(&frame));
        let id = id.to_owned();
        Rc::new(move |height| {
            if let Some(cache) = cache.upgrade()
                && let Some(entry) = cache.borrow_mut().entries.get_mut(&id)
                && let Value::Generated(current) = &entry.value
                && frame
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|frame| Rc::ptr_eq(&frame, current))
            {
                entry.height = height;
            }
            if let Some(updates) = updates.upgrade() {
                updates.changed();
            }
        })
    }

    pub(super) fn answer_failure(&self, id: &str, updates: &Rc<Updates>) -> Rc<dyn Fn()> {
        let cache = Rc::downgrade(&self.inner);
        let updates = Rc::downgrade(updates);
        let frame = self.answer_frame(id).map(|frame| Rc::downgrade(&frame));
        let id = id.to_owned();
        Rc::new(move || {
            if let Some(cache) = cache.upgrade()
                && let Some(entry) = cache.borrow_mut().entries.get_mut(&id)
                && let Value::Generated(current) = &entry.value
                && frame
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|frame| Rc::ptr_eq(&frame, current))
            {
                entry.pump_allowed.set(false);
                entry.value = Value::Failed;
            }
            if let Some(updates) = updates.upgrade() {
                updates.changed();
            }
        })
    }

    pub(super) fn view(
        &self,
        id: &str,
        updates: &Rc<Updates>,
        typography: super::preferences::Typography,
    ) -> El {
        let cache = self.inner.borrow();
        let Some(entry) = cache.entries.get(id) else {
            return typography.text("Loading activity");
        };
        match &entry.value {
            Value::Pending | Value::Loading => typography.text("Loading activity"),
            Value::Waiting(_) => typography.text("Waiting for the current answer"),
            Value::Failed => {
                let mut content = col()
                    .gap(8.0)
                    .child(typography.text("The activity could not be loaded.").wrap());
                content = content.child(self.button(id, "Retry", None, updates, typography));
                content
            }
            Value::Table(table) => table_view(table, typography),
            Value::Chart(chart) => {
                chart_view(chart, typography).child(self.chart_export(id, updates, typography))
            }
            Value::Generated(frame) => col()
                .gap(8.0)
                .child(typography.text("Interactive answer"))
                .child({
                    let id = id.to_owned();
                    let key = format!("answer-open:{id}");
                    let action = Rc::new(move |state: &mut State, _: &mut Tx| {
                        state.sound(crate::interaction_audio::Cue::Press);
                        if state.answers.focus(&id).is_err() {
                            state.fail(FlowError::Host("The answer could not be focused.".into()));
                        }
                    });
                    let press = Rc::clone(&action);
                    typography
                        .text("Interact with answer")
                        .key(key)
                        .role("button")
                        .focusable(true)
                        .press(oklch(0.6, 0.0, 0.0).alpha(0.2))
                        .on_press_tx(move |state: &mut State, tx| press(state, tx))
                        .on_key(move |state: &mut State, key, tx| {
                            if key.key != ::rune::Key::Enter {
                                return false;
                            }
                            if !tx.key_repeat() {
                                action(state, tx);
                            }
                            true
                        })
                })
                .child(if frame.has_content() {
                    ::rune::row()
                        .w_full()
                        .h(entry.height)
                        .drawn_rect_into(&entry.rect)
                } else {
                    typography.text("Waiting for answer content")
                })
                .child(self.answer_export(id, updates, typography)),
            Value::Tools(tools) => match tools.page(entry.offset) {
                Ok(page) => {
                    let mut out = col()
                        .gap(8.0)
                        .child(typography.text(format!("Tools · {}", tools.run_status())));
                    for tool in page.tools() {
                        let mut line = col()
                            .gap(3.0)
                            .child(typography.text(tool.name().unwrap_or_else(|| tool.id())))
                            .child(
                                typography
                                    .text(format!(
                                        "{} {}",
                                        tool.phase().unwrap_or_default(),
                                        tool.outcome().unwrap_or_default()
                                    ))
                                    .wrap(),
                            );
                        for (label, preview) in [
                            ("Title", tool.title()),
                            ("Input", tool.input()),
                            ("Output", tool.output()),
                        ] {
                            if let Some(preview) = preview {
                                line = line.child(
                                    typography
                                        .text(format!(
                                            "{label}: {}{}",
                                            preview.text(),
                                            if preview.truncated() { "…" } else { "" }
                                        ))
                                        .wrap(),
                                );
                            }
                        }
                        for report in tool.history() {
                            line = line.child(
                                typography
                                    .text(format!(
                                        "{} {}",
                                        report.phase(),
                                        report.outcome().unwrap_or_default()
                                    ))
                                    .wrap(),
                            );
                        }
                        out = out.child(line);
                    }
                    self.paging(
                        out,
                        id,
                        entry.offset,
                        page.next_offset(),
                        updates,
                        typography,
                    )
                }
                Err(_) => typography.text("Tool activity is no longer available."),
            },
            Value::Details(details) => match details.page(entry.offset) {
                Ok(page) => {
                    let mut out = col().gap(8.0).child(typography.text(format!(
                        "{} · {}",
                        details.route(),
                        details.status()
                    )));
                    for report in page.reports() {
                        out = out.child(
                            typography
                                .text(format!(
                                    "{} · {} {}",
                                    report.tool(),
                                    report.phase(),
                                    report.outcome().unwrap_or_default()
                                ))
                                .wrap(),
                        );
                    }
                    if let Ok(receipts) = details.corrections() {
                        for receipt in receipts {
                            out = out.child(
                                typography
                                    .text(format!("{}: {}", receipt.label(), receipt.text()))
                                    .wrap(),
                            );
                        }
                    }
                    self.paging(
                        out,
                        id,
                        entry.offset,
                        page.next_offset(),
                        updates,
                        typography,
                    )
                }
                Err(_) => typography.text("Run details are no longer available."),
            },
        }
    }

    fn paging(
        &self,
        mut out: El,
        id: &str,
        offset: u32,
        next: Option<u32>,
        updates: &Rc<Updates>,
        typography: super::preferences::Typography,
    ) -> El {
        if offset > 0 {
            out = out.child(self.button(id, "First page", Some(0), updates, typography));
        }
        if let Some(next) = next {
            out = out.child(self.button(id, "Next page", Some(next), updates, typography));
        }
        out
    }

    fn button(
        &self,
        id: &str,
        label: &'static str,
        offset: Option<u32>,
        updates: &Rc<Updates>,
        typography: super::preferences::Typography,
    ) -> El {
        let cache: Weak<RefCell<Cache>> = Rc::downgrade(&self.inner);
        let updates = Rc::downgrade(updates);
        let id = id.to_owned();
        let action = move |_: &mut State, _: &mut Tx| {
            if let Some(cache) = cache.upgrade()
                && let Some(entry) = cache.borrow_mut().entries.get_mut(&id)
            {
                if let Some(offset) = offset {
                    entry.offset = offset;
                } else {
                    entry.value = Value::Pending;
                }
            }
            if let Some(updates) = updates.upgrade() {
                updates.changed();
            }
        };
        let action = Rc::new(action);
        let press = Rc::clone(&action);
        typography
            .text(label)
            .role("button")
            .focusable(true)
            .on_press_tx(move |s: &mut State, tx| {
                s.sound(crate::interaction_audio::Cue::Press);
                press(s, tx)
            })
            .on_key(move |s: &mut State, key, tx| {
                if key.key == ::rune::Key::Enter {
                    s.sound(crate::interaction_audio::Cue::Press);
                    action(s, tx);
                    true
                } else {
                    false
                }
            })
    }

    fn chart_export(
        &self,
        id: &str,
        updates: &Rc<Updates>,
        typography: super::preferences::Typography,
    ) -> El {
        let cache = Rc::downgrade(&self.inner);
        let updates = Rc::downgrade(updates);
        let id = id.to_owned();
        let action = Rc::new(move |_: &mut State, _: &mut Tx| {
            let Some(cache) = cache.upgrade() else { return };
            let (core, reference) = {
                let cache = cache.borrow();
                let Some(core) = cache.core.upgrade() else {
                    return;
                };
                let Some(Entry {
                    activity: Activity::Chart(reference),
                    ..
                }) = cache.entries.get(&id)
                else {
                    return;
                };
                (core, reference.clone())
            };
            let id = id.clone();
            let updates = updates.clone();
            let cache = Rc::downgrade(&cache);
            spawn_local(async move {
                let bridge = UiBridge { core };
                let result = async {
                    let encoded = bridge
                        .chart_export(
                            reference.run_id.as_str().into(),
                            reference.artifact_id.as_str().into(),
                        )
                        .await?;
                    let result: serde_json::Value = serde_json::from_str(&encoded)
                        .map_err(|_| crate::err("Invalid chart export."))?;
                    bridge.core.ensure_current().map_err(js_error)?;
                    let current = cache.upgrade().is_some_and(|cache| {
                        cache
                            .borrow()
                            .entries
                            .get(&id)
                            .is_some_and(|entry| entry.activity == Activity::Chart(reference))
                    });
                    if !current {
                        return Err(crate::err("Chart changed during export."));
                    }
                    download(
                        result["filename"]
                            .as_str()
                            .ok_or_else(|| crate::err("Chart filename unavailable."))?,
                        result["html"]
                            .as_str()
                            .ok_or_else(|| crate::err("Chart export unavailable."))?,
                    )
                }
                .await;
                if result.is_err()
                    && let Some(updates) = updates.upgrade()
                {
                    updates.finish(Err(FlowError::Host("The chart could not be saved.".into())));
                }
            });
        });
        let press = Rc::clone(&action);
        typography
            .text("Save chart")
            .role("button")
            .focusable(true)
            .on_press_tx(move |s: &mut State, tx| {
                s.sound(crate::interaction_audio::Cue::Press);
                press(s, tx)
            })
            .on_key(move |s: &mut State, key, tx| {
                if key.key == ::rune::Key::Enter {
                    s.sound(crate::interaction_audio::Cue::Press);
                    action(s, tx);
                    true
                } else {
                    false
                }
            })
    }

    fn answer_export(
        &self,
        id: &str,
        updates: &Rc<Updates>,
        typography: super::preferences::Typography,
    ) -> El {
        let cache = Rc::downgrade(&self.inner);
        let updates = Rc::downgrade(updates);
        let id = id.to_owned();
        let action = Rc::new(move |_: &mut State, _: &mut Tx| {
            let Some(cache) = cache.upgrade() else { return };
            let frame = {
                let cache = cache.borrow();
                let Some(Entry {
                    value: Value::Generated(frame),
                    ..
                }) = cache.entries.get(&id)
                else {
                    return;
                };
                Rc::clone(frame)
            };
            let id = id.clone();
            let updates = updates.clone();
            let cache = Rc::downgrade(&cache);
            spawn_local(async move {
                let result = async {
                    let html = frame.export_html().await?;
                    let current = cache.upgrade().is_some_and(|cache| cache.borrow().entries.get(&id)
                        .is_some_and(|entry| matches!(&entry.value, Value::Generated(now) if Rc::ptr_eq(now, &frame))));
                    if !current { return Err(crate::err("Answer changed during export.")); }
                    if let Some(html) = html { download("metldr-answer.html", &html)?; }
                    Ok::<(), JsValue>(())
                }.await;
                if result.is_err()
                    && let Some(updates) = updates.upgrade()
                {
                    updates.finish(Err(FlowError::Host(
                        "The answer could not be saved.".into(),
                    )));
                }
            });
        });
        let press = Rc::clone(&action);
        typography
            .text("Save answer")
            .role("button")
            .focusable(true)
            .on_press_tx(move |s: &mut State, tx| {
                s.sound(crate::interaction_audio::Cue::Press);
                press(s, tx)
            })
            .on_key(move |s: &mut State, key, tx| {
                if key.key == ::rune::Key::Enter {
                    s.sound(crate::interaction_audio::Cue::Press);
                    action(s, tx);
                    true
                } else {
                    false
                }
            })
    }
}

async fn read(
    core: Rc<Core>,
    activity: &Activity,
    existing: Option<Rc<UiAnswerFrame>>,
) -> Result<Value, ()> {
    let bridge = UiBridge { core };
    match activity {
        Activity::Table(reference) => {
            let json = bridge
                .table(
                    reference.run_id.as_str().into(),
                    reference.artifact_id.as_str().into(),
                )
                .await
                .map_err(|_| ())?;
            serde_json::from_str(&json)
                .map(Value::Table)
                .map_err(|_| ())
        }
        Activity::Chart(reference) => {
            let json = bridge
                .chart(
                    reference.run_id.as_str().into(),
                    reference.artifact_id.as_str().into(),
                )
                .await
                .map_err(|_| ())?;
            let value: serde_json::Value = serde_json::from_str(&json).map_err(|_| ())?;
            serde_json::from_value(value["chart"].clone())
                .map(Value::Chart)
                .map_err(|_| ())
        }
        Activity::Generated(reference) => {
            let frame = match existing {
                Some(frame) => frame,
                None => Rc::new(
                    bridge
                        .answer_frame(
                            reference.run_id.as_str().into(),
                            reference.artifact_id.as_str().into(),
                        )
                        .map_err(|_| ())?,
                ),
            };
            let interrupted = frame.completion() == "interrupted";
            if !frame
                .refresh(reference.revision.0.to_string())
                .await
                .map_err(|_| ())?
            {
                return Ok(Value::Waiting(frame));
            }
            if !interrupted && frame.completion() == "interrupted" {
                frame.restart().map_err(|_| ())?;
            }
            Ok(Value::Generated(frame))
        }
        Activity::NativeTools(reference) => {
            let encoded = serde_json::to_string(reference).map_err(|_| ())?;
            bridge
                .native_tools(encoded)
                .await
                .map(Value::Tools)
                .map_err(|_| ())
        }
        Activity::RunDetails(reference) => {
            let encoded = serde_json::to_string(reference).map_err(|_| ())?;
            bridge
                .run_details(encoded)
                .await
                .map(Value::Details)
                .map_err(|_| ())
        }
    }
}

fn table_view(table: &Table, typography: super::preferences::Typography) -> El {
    const COLUMN_WIDTH: f32 = 160.0;
    let mut grid = col()
        .role("table")
        .label(table.title())
        .w(COLUMN_WIDTH * table.columns().len() as f32);
    let mut header = ::rune::row().role("row").w_full();
    for column in table.columns() {
        header = header.child(
            typography
                .text(column)
                .role("columnheader")
                .w(COLUMN_WIDTH)
                .padding(6.0)
                .font(typography.font(14.0, 600.0))
                .wrap(),
        );
    }
    grid = grid.child(header);
    for values in table.rows() {
        let mut row = ::rune::row().role("row").w_full();
        for (column, value) in table.columns().iter().zip(values) {
            row = row.child(
                typography
                    .text(value)
                    .role("cell")
                    .label(format!("{column}: {value}"))
                    .w(COLUMN_WIDTH)
                    .padding(6.0)
                    .wrap()
                    .selectable(true),
            );
        }
        grid = grid.child(row);
    }
    let mut out = col()
        .gap(6.0)
        .child(typography.text(table.title()).role("heading").wrap())
        .child(::rune::row().w_full().scroll_x().child(grid));
    if table.rows().is_empty() {
        out = out.child(typography.text("No rows to display."));
    }
    out.child(typography.text(table.source()).wrap())
}

fn chart_view(chart: &Chart, typography: super::preferences::Typography) -> El {
    let mut out = col()
        .gap(6.0)
        .child(typography.text(chart.title()).role("heading").wrap())
        .child(typography.text(chart.description()).wrap());
    match chart.kind() {
        ChartKind::Bar => {
            let (min, max) = chart
                .data()
                .iter()
                .filter_map(|p| p.value.as_f64())
                .fold((0.0_f64, 0.0_f64), |(min, max), value| {
                    (min.min(value), max.max(value))
                });
            let span = max - min;
            let x = |value: f64| {
                if span > 0.0 {
                    ((value - min) / span * 200.0) as f32
                } else {
                    100.0
                }
            };
            let zero = x(0.0);
            out = out.child(
                typography
                    .text(format!(
                        "Scale: {} to {}; zero marked",
                        chart_scale(min),
                        chart_scale(max)
                    ))
                    .wrap(),
            );
            for point in chart.data() {
                out = out.child(
                    typography
                        .text(format!("{}: {}", point.label, point.value))
                        .wrap()
                        .selectable(true),
                );
                if let Some(value) = point.value.as_f64() {
                    let end = x(value);
                    let mut shape = ::rune::Shape::new();
                    if end != zero {
                        shape = shape.fill(
                            vec![
                                [zero.min(end), 3.0],
                                [zero.max(end), 3.0],
                                [zero.max(end), 13.0],
                                [zero.min(end), 13.0],
                            ],
                            oklch(0.65, 0.16, 250.0),
                        );
                    }
                    shape = shape.stroke(vec![[zero, 0.0], [zero, 16.0]], 1.0, typography.ink);
                    out = out.child(
                        ::rune::canvas(shape.view_box(200.0, 16.0))
                            .w_full()
                            .h(16.0)
                            .label(format!("{}: {}", point.label, point.value)),
                    );
                }
            }
        }
        ChartKind::Pie => {
            let total: f64 = chart.data().iter().filter_map(|p| p.value.as_f64()).sum();
            if total > 0.0 {
                let mut shape = ::rune::Shape::new();
                let mut angle = -std::f64::consts::FRAC_PI_2;
                for (index, point) in chart.data().iter().enumerate() {
                    let Some(value) = point.value.as_f64() else {
                        continue;
                    };
                    let end = angle + std::f64::consts::TAU * (value / total);
                    if end > angle {
                        let steps =
                            (((end - angle) / std::f64::consts::TAU * 64.0).ceil() as usize).max(1);
                        let color = oklch(0.7, 0.14, (index * 47 % 360) as f32);
                        for i in 0..steps {
                            let a = angle + (end - angle) * i as f64 / steps as f64;
                            let b = angle + (end - angle) * (i + 1) as f64 / steps as f64;
                            shape = shape.fill(
                                vec![
                                    [80.0, 80.0],
                                    [80.0 + 72.0 * a.cos() as f32, 80.0 + 72.0 * a.sin() as f32],
                                    [80.0 + 72.0 * b.cos() as f32, 80.0 + 72.0 * b.sin() as f32],
                                ],
                                color,
                            );
                        }
                    }
                    angle = end;
                }
                out = out.child(::rune::canvas(shape).size(160.0, 160.0).label("Pie chart"));
            } else if !chart.data().is_empty() {
                out = out.child(typography.text("All values are zero."));
            }
            for (index, point) in chart.data().iter().enumerate() {
                out = out.child(
                    ::rune::row()
                        .gap(6.0)
                        .center_y()
                        .child(::rune::row().size(10.0, 10.0).fill(oklch(
                            0.7,
                            0.14,
                            (index * 47 % 360) as f32,
                        )))
                        .child(
                            typography
                                .text(format!("{}: {}", point.label, point.value))
                                .wrap()
                                .selectable(true),
                        ),
                );
            }
        }
    }
    if chart.data().is_empty() {
        out = out.child(typography.text("No data to display."));
    }
    out.child(typography.text(chart.source()))
}

fn chart_scale(value: f64) -> String {
    if value != 0.0 && !(0.001..1_000_000.0).contains(&value.abs()) {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
}

fn download(filename: &str, html: &str) -> Result<(), JsValue> {
    let global = js_sys::global();
    let blob = js_sys::Reflect::get(&global, &"Blob".into())?.dyn_into::<js_sys::Function>()?;
    let parts = js_sys::Array::new();
    parts.push(&html.into());
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"type".into(), &"text/html;charset=utf-8".into())?;
    let args = js_sys::Array::new();
    args.push(&parts);
    args.push(&options);
    let blob = js_sys::Reflect::construct(&blob, &args)?;
    let url_api = js_sys::Reflect::get(&global, &"URL".into())?;
    let url = method(&url_api, "createObjectURL")?
        .call1(&url_api, &blob)?
        .as_string()
        .ok_or_else(|| crate::err("Chart URL unavailable."))?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| crate::err("Document unavailable."))?;
    let anchor = document
        .create_element("a")?
        .dyn_into::<web_sys::HtmlElement>()?;
    anchor.set_attribute("href", &url)?;
    anchor.set_attribute("download", filename)?;
    document
        .body()
        .ok_or_else(|| crate::err("Document body unavailable."))?
        .append_child(&anchor)?;
    anchor.click();
    anchor.remove();
    let revoke = Closure::once_into_js(move || {
        if let Ok(f) = method(&url_api, "revokeObjectURL") {
            let _ = f.call1(&url_api, &url.into());
        }
    });
    web_sys::window()
        .ok_or_else(|| crate::err("Window unavailable."))?
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 0)?;
    Ok(())
}

/// Owns the existing answer protocol over a sandboxed iframe and private MessagePort.
/// The host positions and clips `container` to the corresponding Rune activity.
pub(super) struct AnswerMount(Rc<AnswerTransport>);

struct AnswerTransport {
    answer: Rc<UiAnswerFrame>,
    initialization: String,
    pump_allowed: Rc<Cell<bool>>,
    iframe: web_sys::Element,
    port: JsValue,
    load: RefCell<Option<Closure<dyn FnMut(web_sys::Event)>>>,
    message: RefCell<Option<Closure<dyn FnMut(JsValue)>>>,
    timeout: RefCell<Option<Closure<dyn FnMut()>>>,
    timeout_id: std::cell::Cell<Option<i32>>,
    closed: std::cell::Cell<bool>,
    loaded: std::cell::Cell<bool>,
    followup: Rc<dyn Fn(String)>,
    failure: Rc<dyn Fn()>,
    height: Rc<dyn Fn(f32)>,
    wheel: Rc<dyn Fn(f32, f32)>,
}

fn method(object: &JsValue, name: &str) -> Result<js_sys::Function, JsValue> {
    js_sys::Reflect::get(object, &name.into())?.dyn_into()
}

impl AnswerMount {
    pub(super) fn attach(
        container: &web_sys::Element,
        answer: Rc<UiAnswerFrame>,
        followup: Rc<dyn Fn(String)>,
        failure: Rc<dyn Fn()>,
        height: Rc<dyn Fn(f32)>,
        pump_allowed: Rc<Cell<bool>>,
        wheel: Rc<dyn Fn(f32, f32)>,
    ) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| crate::err("Window unavailable."))?;
        let document = window
            .document()
            .ok_or_else(|| crate::err("Document unavailable."))?;
        container
            .clone()
            .dyn_into::<web_sys::HtmlElement>()?
            .style()
            .set_property("display", "none")?;
        let url =
            web_sys::Url::new_with_base("/answer-sandbox.html", &window.location().href()?)?.href();
        if document
            .query_selector("meta[data-answer-frame-policy]")?
            .is_none()
        {
            let policy = document.create_element("meta")?;
            policy.set_attribute("http-equiv", "Content-Security-Policy")?;
            policy.set_attribute("content", &format!("frame-src {url}"))?;
            policy.set_attribute("data-answer-frame-policy", "")?;
            document
                .query_selector("head")?
                .ok_or_else(|| crate::err("Document head unavailable."))?
                .append_child(&policy)?;
        }
        let iframe = document.create_element("iframe")?;
        iframe.set_attribute("title", "Interactive answer")?;
        iframe.set_attribute("sandbox", "allow-scripts")?;
        iframe.set_attribute("referrerpolicy", "no-referrer")?;
        iframe.set_attribute("width", "100%")?;
        iframe.set_attribute("height", &answer.height().to_string())?;
        let channel = js_sys::Reflect::get(&js_sys::global(), &"MessageChannel".into())?
            .dyn_into::<js_sys::Function>()?;
        let channel = js_sys::Reflect::construct(&channel, &js_sys::Array::new())?;
        let port = js_sys::Reflect::get(&channel, &"port1".into())?;
        let peer = js_sys::Reflect::get(&channel, &"port2".into())?;
        let initialization = answer.initialize()?;
        let inner = Rc::new(AnswerTransport {
            answer,
            initialization,
            pump_allowed,
            iframe,
            port,
            load: RefCell::new(None),
            message: RefCell::new(None),
            timeout: RefCell::new(None),
            timeout_id: std::cell::Cell::new(None),
            closed: std::cell::Cell::new(false),
            loaded: std::cell::Cell::new(false),
            followup,
            failure,
            height,
            wheel,
        });
        let weak = Rc::downgrade(&inner);
        let on_message = Closure::new(move |event: JsValue| {
            let Some(inner) = weak.upgrade() else { return };
            if inner.closed.get() || !inner.is_current() {
                return;
            }
            let result = (|| -> Result<(), JsValue> {
                let encoded = js_sys::Reflect::get(&event, &"data".into())?
                    .as_string()
                    .ok_or_else(|| crate::err("Invalid answer message."))?;
                let event = inner.answer.receive(encoded)?;
                match event.kind() {
                    metldr_answer_codec::AnswerEventKind::Ready
                    | metldr_answer_codec::AnswerEventKind::Ack => inner.cancel_timer(),
                    metldr_answer_codec::AnswerEventKind::Height => {
                        let height = event.height()? as f32;
                        inner
                            .iframe
                            .set_attribute("height", &format!("{}", height as u16))?;
                        (inner.height)(height);
                    }
                    metldr_answer_codec::AnswerEventKind::Followup => {
                        (inner.followup)(event.text()?)
                    }
                    metldr_answer_codec::AnswerEventKind::Wheel => {
                        (inner.wheel)(event.wheel_dx()? as f32, event.wheel_dy()? as f32);
                    }
                    metldr_answer_codec::AnswerEventKind::Failed
                    | metldr_answer_codec::AnswerEventKind::RuntimeFailed => {
                        return Err(crate::err("Answer failed."));
                    }
                }
                inner.pump()
            })();
            if result.is_err() {
                inner.fail();
            }
        });
        js_sys::Reflect::set(&inner.port, &"onmessage".into(), on_message.as_ref())?;
        method(&inner.port, "start")?.call0(&inner.port)?;
        *inner.message.borrow_mut() = Some(on_message);
        let weak = Rc::downgrade(&inner);
        let on_load = Closure::new(move |_: web_sys::Event| {
            let Some(inner) = weak.upgrade() else { return };
            if inner.closed.get() || !inner.is_current() {
                return;
            }
            if inner.loaded.replace(true) {
                inner.fail();
                return;
            }
            let result = (|| -> Result<(), JsValue> {
                let target = js_sys::Reflect::get(inner.iframe.as_ref(), &"contentWindow".into())?;
                if target.is_null() || target.is_undefined() {
                    return Err(crate::err("Answer frame unavailable."));
                }
                let transfer = js_sys::Array::new();
                transfer.push(&peer);
                method(&target, "postMessage")?.call3(
                    &target,
                    &inner.initialization.clone().into(),
                    &"*".into(),
                    &transfer,
                )?;
                Ok(())
            })();
            if result.is_err() {
                inner.fail();
            }
        });
        inner
            .iframe
            .add_event_listener_with_callback("load", on_load.as_ref().unchecked_ref())?;
        *inner.load.borrow_mut() = Some(on_load);
        inner.arm()?;
        inner.iframe.set_attribute("src", &url)?;
        container.append_child(&inner.iframe)?;
        Ok(Self(inner))
    }

    pub(super) fn consume_followup(&self) -> Result<(), JsValue> {
        self.0.answer.consume_followup()?;
        self.0.pump()
    }

    pub(super) fn matches_controller(&self, answer: &Rc<UiAnswerFrame>) -> bool {
        Rc::ptr_eq(&self.0.answer, answer) && self.0.is_current()
    }

    /// Call after `Activities::sync` and on the wake following a completed refresh.
    pub(super) fn pump(&self) -> Result<(), JsValue> {
        self.0.pump()
    }

    /// Position inside an independently measured scroll viewport; no unbounded overlay.
    pub(super) fn position(&self, bounds: [f32; 4], clip: [f32; 4]) -> Result<(), JsValue> {
        if bounds.iter().chain(clip.iter()).any(|v| !v.is_finite()) {
            return Err(crate::err("Invalid answer bounds."));
        }
        let (x, y, w, h) = (bounds[0], bounds[1], bounds[2].max(0.0), bounds[3].max(0.0));
        let left = x.max(clip[0]);
        let top = y.max(clip[1]);
        let right = (x + w).min(clip[0] + clip[2].max(0.0));
        let bottom = (y + h).min(clip[1] + clip[3].max(0.0));
        let host = self
            .0
            .iframe
            .parent_element()
            .ok_or_else(|| crate::err("Answer host unavailable."))?
            .dyn_into::<web_sys::HtmlElement>()?;
        let style = host.style();
        for (name, value) in [
            ("position", "fixed".to_owned()),
            ("overflow", "hidden".to_owned()),
            (
                "display",
                if right > left && bottom > top {
                    "block"
                } else {
                    "none"
                }
                .to_owned(),
            ),
            ("left", format!("{left}px")),
            ("top", format!("{top}px")),
            ("width", format!("{}px", (right - left).max(0.0))),
            ("height", format!("{}px", (bottom - top).max(0.0))),
        ] {
            style.set_property(name, &value)?;
        }
        let frame = self.0.iframe.clone().dyn_into::<web_sys::HtmlElement>()?;
        let style = frame.style();
        for (name, value) in [
            ("position", "absolute".to_owned()),
            ("left", format!("{}px", x - left)),
            ("top", format!("{}px", y - top)),
            ("width", format!("{w}px")),
            ("height", format!("{h}px")),
        ] {
            style.set_property(name, &value)?;
        }
        Ok(())
    }
}

impl AnswerTransport {
    fn is_current(&self) -> bool {
        self.answer
            .initialize()
            .is_ok_and(|binding| binding == self.initialization)
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.timeout_id.take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(id);
        }
    }

    fn arm(self: &Rc<Self>) -> Result<(), JsValue> {
        self.cancel_timer();
        let weak = Rc::downgrade(self);
        let timeout = Closure::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.fail();
            }
        });
        let window = web_sys::window().ok_or_else(|| crate::err("Window unavailable."))?;
        let id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            timeout.as_ref().unchecked_ref(),
            5_000,
        )?;
        *self.timeout.borrow_mut() = Some(timeout);
        self.timeout_id.set(Some(id));
        Ok(())
    }

    fn pump(self: &Rc<Self>) -> Result<(), JsValue> {
        if self.closed.get() {
            return Err(crate::err("Answer closed."));
        }
        if !self.pump_allowed.get() {
            return Ok(());
        }
        if let Some(command) = self.answer.next_command()? {
            self.arm()?;
            method(&self.port, "postMessage")?.call1(&self.port, &command.into())?;
        }
        Ok(())
    }

    fn fail(&self) {
        let current = self.is_current();
        if current {
            self.answer.close();
        }
        self.close();
        if current {
            (self.failure)();
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.cancel_timer();
        let _ = js_sys::Reflect::set(&self.port, &"onmessage".into(), &JsValue::NULL);
        if let Ok(close) = method(&self.port, "close") {
            let _ = close.call0(&self.port);
        }
        if let Some(load) = self.load.borrow().as_ref() {
            let _ = self
                .iframe
                .remove_event_listener_with_callback("load", load.as_ref().unchecked_ref());
        }
        self.iframe.remove();
    }
}

impl Drop for AnswerMount {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl Drop for AnswerTransport {
    fn drop(&mut self) {
        self.close();
    }
}
