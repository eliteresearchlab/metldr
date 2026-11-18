//! The Stats screen: what metldr did for the reader over a period, from the extension's local
//! records. It reads once when opened or when the period changes, never on a timer.

use super::preferences::{Palette, Typography};
use super::*;
use ::rune::{El, col, row};
use serde_json::json;

/// The periods the screen offers, in days.
const PERIODS: [(u32, &str); 3] = [(7, "7 days"), (30, "30 days"), (90, "90 days")];
const CHART_HEIGHT: f32 = 72.0;

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Summary {
    summaries: f64,
    chat_messages: f64,
    lookups: f64,
    words_read: f64,
    time_saved_seconds: f64,
    pages: f64,
    pdfs: f64,
    emails: f64,
    tokens_input: f64,
    tokens_output: f64,
    avg_response_ms: Option<f64>,
    fastest_response_ms: Option<f64>,
    current_streak: u32,
    longest_streak: u32,
    per_day: Vec<f64>,
    models: Vec<(String, f64)>,
    top_domains: Vec<(String, f64)>,
}

pub(super) struct Stats {
    days: u32,
    summary: Option<Summary>,
    inbox: Rc<RefCell<Option<Summary>>>,
    wake: Option<::rune::Waker>,
}

impl Default for Stats {
    fn default() -> Self {
        Self { days: 30, summary: None, inbox: Rc::default(), wake: None }
    }
}

impl Stats {
    pub(super) fn setup(&mut self, wake: ::rune::Waker) {
        self.wake = Some(wake);
    }

    /// Reads the summary for the current period.
    pub(super) fn load(&mut self) {
        let (inbox, wake, days) = (Rc::clone(&self.inbox), self.wake.clone(), self.days);
        spawn_local(async move {
            let reply = gateway::ask_extension(&json!({"type": "STATS_SUMMARY", "days": days})).await;
            let summary = reply.and_then(|r| serde_json::from_value(r).ok()).unwrap_or_default();
            *inbox.borrow_mut() = Some(summary);
            if let Some(wake) = &wake {
                wake();
            }
        });
    }

    pub(super) fn poll(&mut self) -> bool {
        let next = self.inbox.borrow_mut().take();
        let changed = next.is_some();
        if next.is_some() {
            self.summary = next;
        }
        changed
    }

    fn period(&mut self, days: u32) {
        if self.days != days {
            self.days = days;
            self.load();
        }
    }

    pub(super) fn view(&self, typography: Typography, palette: &Palette) -> El {
        let choices = PERIODS.into_iter().map(|(days, label)| {
            let checked = self.days == days;
            let key = format!("stats-period-{days}");
            let el = super::preferences::outlined(typography, label, key)
                .border(if checked { 1.5 } else { 1.0 }, if checked { typography.ink } else { typography.line });
            metldr_ui::choice::radio(el, false, checked, move |state: &mut State, _| state.stats.period(days))
                .press(typography.press)
        });
        let group = metldr_ui::choice::GroupProps { axis: ::rune::FocusAxis::Horizontal, ..Default::default() };
        let periods = metldr_ui::choice::radio_group(row().key("stats-period").gap(6.0), "Period", group);
        let mut out = col().w_full().gap(12.0).child(periods.children(choices));
        let Some(s) = &self.summary else {
            return out.child(typography.text("Loading stats.").key("stats-status").role("status"));
        };
        let tiles = [
            ("Summaries", count(s.summaries)),
            ("Chat messages", count(s.chat_messages)),
            ("Words looked up", count(s.lookups)),
            ("Words read", count(s.words_read)),
            ("Reading time saved", duration(s.time_saved_seconds * 1000.0)),
            ("Pages, PDFs, emails", format!("{}, {}, {}", count(s.pages), count(s.pdfs), count(s.emails))),
            ("Tokens in, out", format!("{}, {}", count(s.tokens_input), count(s.tokens_output))),
            ("Average answer", s.avg_response_ms.map_or("None yet".into(), duration)),
            ("Fastest answer", s.fastest_response_ms.map_or("None yet".into(), duration)),
            ("Streak", format!("{} days, longest {}", s.current_streak, s.longest_streak)),
        ];
        let tiles = tiles.into_iter().map(|(label, value)| tile(typography, palette, label, value));
        out = out.child(row().key("stats-tiles").w_full().gap(8.0).flex_wrap().children(tiles));
        out = out.child(chart(typography, palette, &s.per_day));
        for (title, list) in [("Models", &s.models), ("Sites", &s.top_domains)] {
            if list.is_empty() {
                continue;
            }
            let rows = list.iter().map(|(name, n)| {
                row().w_full().gap(8.0)
                    .child(typography.text(name.as_str()).grow(1.0).truncate())
                    .child(typography.text(count(*n)).color(palette.ink_2))
            });
            out = out.child(typography.text(title).role("heading")).child(col().w_full().gap(4.0).children(rows));
        }
        out
    }
}

/// One number with its label.
fn tile(typography: Typography, palette: &Palette, label: &str, value: String) -> El {
    let label_text = format!("{label}: {value}");
    col()
        .min_w(140.0)
        .grow(1.0)
        .gap(2.0)
        .padding(10.0)
        .radius(8.0)
        .fill(palette.field)
        .label(label_text)
        .child(typography.legend(label).font(typography.font(11.5, 500.0)).color(palette.ink_2).a11y_hidden())
        .child(typography.legend(value).font(typography.font(16.0, 600.0).features("'tnum' 1")).a11y_hidden())
}

/// Interactions per day as bars, oldest on the left, named in words for screen readers.
fn chart(typography: Typography, palette: &Palette, per_day: &[f64]) -> El {
    let most = per_day.iter().copied().fold(0.0, f64::max);
    let total: f64 = per_day.iter().sum();
    let bars = per_day.iter().map(|&n| {
        let h = if most > 0.0 { (n / most) as f32 * CHART_HEIGHT } else { 0.0 };
        col().grow(1.0).h(CHART_HEIGHT).justify_end().child(
            ::rune::el().w_full().h(h.max(if n > 0.0 { 2.0 } else { 1.0 })).radius(2.0)
                .fill(if n > 0.0 { palette.accent } else { palette.line }),
        )
    });
    let label = format!(
        "Interactions per day over {} days: {} in all, at most {} in a day",
        per_day.len(), count(total), count(most)
    );
    col().w_full().gap(6.0)
        .child(typography.text("Activity").role("heading"))
        .child(row().key("stats-chart").w_full().gap(2.0).items_end().role("img").label(label).children(bars.map(El::a11y_hidden)))
}

/// A count as people read it: 980, 1.25K, 12.5M, 125B.
fn count(n: f64) -> String {
    let n = n.max(0.0);
    let (value, unit) = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")]
        .into_iter()
        .find(|(scale, _)| n >= *scale)
        .map_or((n, ""), |(scale, unit)| (n / scale, unit));
    if unit.is_empty() {
        return format!("{}", value.round());
    }
    let digits = if value >= 100.0 { 0 } else if value >= 10.0 { 1 } else { 2 };
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
    format!("{text}{unit}")
}

/// A duration as people read it: 850 ms, 4.2 s, 3 min 12 s, 1 h 05 min.
fn duration(ms: f64) -> String {
    let ms = ms.max(0.0);
    if ms < 1000.0 {
        return format!("{} ms", ms.round());
    }
    let s = ms / 1000.0;
    if s < 60.0 {
        return format!("{s:.1} s");
    }
    let s = s.round() as u64;
    if s < 3600 {
        return format!("{} min {} s", s / 60, s % 60);
    }
    format!("{} h {:02} min", s / 3600, s % 3600 / 60)
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn numbers_read_as_people_read_them() {
        assert_eq!([count(980.0), count(1250.0), count(12_500_000.0), count(125e9)], ["980", "1.25K", "12.5M", "125B"]);
        assert_eq!(count(2000.0), "2K");
        assert_eq!([duration(850.0), duration(4200.0), duration(192_000.0), duration(3_900_000.0)], ["850 ms", "4.2 s", "3 min 12 s", "1 h 05 min"]);
    }
}
