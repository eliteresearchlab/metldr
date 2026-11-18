//! Local usage stats: the summary the Stats page shows, made from the per-day records the
//! extension keeps in IndexedDB (`metldr-analytics`, store `daily_stats`). Nothing leaves the
//! device.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Reading speed behind "time saved": words read for the reader, at this many a minute.
const WORDS_PER_MINUTE: f64 = 200.0;
const TOP: usize = 5;

/// One day's record, as both the TypeScript tracker and [`record_chat`] write it. Unknown
/// fields are kept by the writer and ignored here.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Day {
    pub date: String,
    pub time_in_panel: f64,
    pub summaries_generated: f64,
    pub chat_messages_sent: f64,
    pub chat_messages_received: f64,
    pub words_looked_up: f64,
    pub tokens_input: f64,
    pub tokens_output: f64,
    pub emails_processed: f64,
    pub pages_processed: f64,
    pub pdfs_processed: f64,
    pub total_words_read: f64,
    pub avg_response_time_ms: f64,
    pub response_time_samples: f64,
    /// Stored as Infinity until a response is timed, which reads back as null.
    pub fastest_response_ms: Option<f64>,
    pub hourly_activity: Vec<f64>,
    pub top_domains: HashMap<String, f64>,
    pub model_usage: HashMap<String, f64>,
}

impl Day {
    fn interactions(&self) -> f64 {
        self.summaries_generated
            + self.chat_messages_sent
            + self.chat_messages_received
            + self.words_looked_up
    }

    fn active(&self) -> bool {
        self.time_in_panel > 0.0 || self.interactions() > 0.0
    }
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub days: u32,
    pub summaries: f64,
    pub chat_messages: f64,
    pub lookups: f64,
    pub words_read: f64,
    pub time_saved_seconds: f64,
    pub pages: f64,
    pub pdfs: f64,
    pub emails: f64,
    pub tokens_input: f64,
    pub tokens_output: f64,
    pub avg_response_ms: Option<f64>,
    pub fastest_response_ms: Option<f64>,
    pub current_streak: u32,
    pub longest_streak: u32,
    /// The hour with the most activity, 0 to 23, when any hour has some.
    pub peak_hour: Option<u8>,
    /// Interactions on each day of the period, oldest first.
    pub per_day: Vec<f64>,
    pub top_domains: Vec<(String, f64)>,
    pub models: Vec<(String, f64)>,
}

/// The summary of the `period` days up to `today` (YYYY-MM-DD, UTC), from every stored day;
/// streaks look at all of them.
pub fn summarize(all: &[Day], today: &str, period: u32) -> Summary {
    let Some(today) = day_number(today) else {
        return Summary::default();
    };
    let period = period.clamp(1, 400);
    let first = today - i64::from(period) + 1;
    let mut out = Summary {
        days: period,
        per_day: vec![0.0; period as usize],
        ..Summary::default()
    };
    let mut weighted = 0.0;
    let mut samples = 0.0;
    let mut hours = [0.0; 24];
    let mut domains: HashMap<&str, f64> = HashMap::new();
    let mut models: HashMap<&str, f64> = HashMap::new();
    let mut active: Vec<i64> = Vec::with_capacity(all.len());
    for day in all {
        let Some(n) = day_number(&day.date) else {
            continue;
        };
        if day.active() {
            active.push(n);
        }
        if n < first || n > today {
            continue;
        }
        out.per_day[(n - first) as usize] += day.interactions();
        out.summaries += day.summaries_generated;
        out.chat_messages += day.chat_messages_sent + day.chat_messages_received;
        out.lookups += day.words_looked_up;
        out.words_read += day.total_words_read;
        out.pages += day.pages_processed;
        out.pdfs += day.pdfs_processed;
        out.emails += day.emails_processed;
        out.tokens_input += day.tokens_input;
        out.tokens_output += day.tokens_output;
        if day.response_time_samples > 0.0 {
            weighted += day.avg_response_time_ms * day.response_time_samples;
            samples += day.response_time_samples;
            if let Some(fastest) = day.fastest_response_ms.filter(|ms| ms.is_finite()) {
                out.fastest_response_ms =
                    Some(out.fastest_response_ms.map_or(fastest, |f| f.min(fastest)));
            }
        }
        for (slot, value) in hours.iter_mut().zip(&day.hourly_activity) {
            *slot += value;
        }
        for (name, count) in &day.top_domains {
            *domains.entry(name).or_default() += count;
        }
        for (name, count) in &day.model_usage {
            *models.entry(name).or_default() += count;
        }
    }
    out.time_saved_seconds = out.words_read / WORDS_PER_MINUTE * 60.0;
    out.avg_response_ms = (samples > 0.0).then(|| weighted / samples);
    out.peak_hour = hours
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.0)
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(h, _)| h as u8);
    out.top_domains = top(domains);
    out.models = top(models);
    (out.current_streak, out.longest_streak) = streaks(active, today);
    out
}

/// The largest counts first, at most [`TOP`], ties by name.
fn top(counts: HashMap<&str, f64>) -> Vec<(String, f64)> {
    let mut list: Vec<_> = counts.into_iter().filter(|(_, c)| *c > 0.0).collect();
    list.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    list.into_iter()
        .take(TOP)
        .map(|(n, c)| (n.to_owned(), c))
        .collect()
}

/// The run of active days ending today or yesterday, and the longest run.
fn streaks(mut active: Vec<i64>, today: i64) -> (u32, u32) {
    active.sort_unstable();
    active.dedup();
    let (mut longest, mut run, mut last) = (0u32, 0u32, i64::MIN);
    for &n in &active {
        run = if n == last + 1 { run + 1 } else { 1 };
        longest = longest.max(run);
        last = n;
    }
    let current = if last >= today - 1 { run } else { 0 };
    (current, longest)
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date (Howard Hinnant's days-from-civil).
pub fn day_number(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let y = num(&date[..4])?;
    let m = num(&date[5..7])?;
    let d = num(&date[8..])?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// One answered chat added to `day` (a stored record, or null for a new day `date`): a message
/// each way, its tokens when the route reported them, its time to the full answer and its model.
/// Every field the tracker writes is kept, so both writers share the record.
pub fn record_chat(
    day: serde_json::Value,
    date: &str,
    tokens: Option<(u64, u64)>,
    ms: f64,
    model: &str,
) -> serde_json::Value {
    use serde_json::{Value, json};
    let mut day = match day {
        Value::Object(map) => Value::Object(map),
        _ => json!({
            "date": date, "timeInPanel": 0, "timeInGmail": 0, "timeInArticles": 0,
            "timeInSettings": 0, "timeInStats": 0, "hourlyActivity": vec![0u8; 24],
            "summariesGenerated": 0, "chatMessagesSent": 0, "chatMessagesReceived": 0,
            "wordsLookedUp": 0, "replySuggestionsGenerated": 0, "replySuggestionsUsed": 0,
            "tokensInput": 0, "tokensOutput": 0, "emailsProcessed": 0, "pagesProcessed": 0,
            "pdfsProcessed": 0, "totalWordsRead": 0, "totalWordsSummarized": 0,
            "avgResponseTimeMs": 0, "responseTimeSamples": 0, "fastestResponseMs": null,
            "slowestResponseMs": 0, "topDomains": {}, "modelUsage": {}, "errorCount": 0,
            "usedChat": false, "usedSummary": false, "usedWordLookup": false
        }),
    };
    let number = |day: &Value, key: &str| day[key].as_f64().unwrap_or(0.0);
    let add = |day: &mut Value, key: &str, by: f64| day[key] = json!(number(day, key) + by);
    add(&mut day, "chatMessagesSent", 1.0);
    add(&mut day, "chatMessagesReceived", 1.0);
    if let Some((input, output)) = tokens {
        add(&mut day, "tokensInput", input as f64);
        add(&mut day, "tokensOutput", output as f64);
    }
    let samples = number(&day, "responseTimeSamples");
    let average = number(&day, "avgResponseTimeMs");
    day["avgResponseTimeMs"] = json!((average * samples + ms) / (samples + 1.0));
    day["responseTimeSamples"] = json!(samples + 1.0);
    let fastest = day["fastestResponseMs"]
        .as_f64()
        .filter(|f| f.is_finite())
        .map_or(ms, |f| f.min(ms));
    day["fastestResponseMs"] = json!(fastest);
    day["slowestResponseMs"] = json!(number(&day, "slowestResponseMs").max(ms));
    day["usedChat"] = json!(true);
    if !model.is_empty() {
        let used = day["modelUsage"][model].as_f64().unwrap_or(0.0);
        if !day["modelUsage"].is_object() {
            day["modelUsage"] = json!({});
        }
        day["modelUsage"][model] = json!(used + 1.0);
    }
    day
}

#[cfg(test)]
mod checks {
    use super::*;
    use serde_json::json;

    fn day(value: serde_json::Value) -> Day {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_period_sums_its_days_and_streaks_count_every_day() {
        let all = [
            day(json!({"date":"2026-11-10","summariesGenerated":1})),
            day(
                json!({"date":"2026-11-16","chatMessagesSent":2,"chatMessagesReceived":2,
                "totalWordsRead":400,"avgResponseTimeMs":900,"responseTimeSamples":2,
                "fastestResponseMs":null,"modelUsage":{"m":2}}),
            ),
            day(
                json!({"date":"2026-11-17","wordsLookedUp":3,"avgResponseTimeMs":300,
                "responseTimeSamples":1,"fastestResponseMs":250,"hourlyActivity":[0,5]}),
            ),
        ];
        let s = summarize(&all, "2026-11-18", 7);
        assert_eq!(s.per_day, [0.0, 0.0, 0.0, 0.0, 4.0, 3.0, 0.0]);
        assert_eq!((s.chat_messages, s.lookups, s.summaries), (4.0, 3.0, 0.0));
        assert_eq!(s.time_saved_seconds, 120.0);
        assert_eq!(s.avg_response_ms, Some(700.0));
        assert_eq!(s.fastest_response_ms, Some(250.0));
        assert_eq!(
            (s.current_streak, s.longest_streak, s.peak_hour),
            (2, 2, Some(1))
        );
        assert_eq!(s.models, [("m".to_owned(), 2.0)]);
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2000-03-01"), Some(11_017));
    }

    #[test]
    fn a_recorded_chat_keeps_the_trackers_fields() {
        let new = record_chat(json!(null), "2026-11-18", Some((100, 20)), 800.0, "m");
        assert_eq!(new["date"], "2026-11-18");
        assert_eq!(new["hourlyActivity"].as_array().unwrap().len(), 24);
        assert_eq!(
            (
                new["chatMessagesSent"].as_f64(),
                new["tokensInput"].as_f64()
            ),
            (Some(1.0), Some(100.0))
        );
        let again = record_chat(new, "2026-11-18", None, 400.0, "m");
        assert_eq!(again["avgResponseTimeMs"], 600.0);
        assert_eq!(
            (
                again["fastestResponseMs"].as_f64(),
                again["modelUsage"]["m"].as_f64()
            ),
            (Some(400.0), Some(2.0))
        );
    }
}
