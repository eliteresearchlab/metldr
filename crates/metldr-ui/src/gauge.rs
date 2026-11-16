//! Meters and progress bars: a number between a minimum and a maximum, drawn as a fill and
//! announced.
//!
//! - A meter always has a reading. Outside its range the fill and the announced number are
//!   clamped; the app keeps the value it gave.
//! - A progress bar has a value while the task can say how far it has come, and none while it
//!   cannot: it is indeterminate, and announces no number.
//! - Each announces its number as a whole percentage unless the app gives its own words, such
//!   as "3 of 5 files".

use rune::El;

/// How much of the fill `value` covers between `min` and `max`, clamped to 0 to 1. A range
/// with no width is empty.
pub fn fraction(value: f64, min: f64, max: f64) -> f32 {
    if max <= min || !value.is_finite() {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
}

/// The words for a fraction: a whole percentage.
pub fn percent(fraction: f32) -> String {
    format!("{}%", (fraction * 100.0).round() as u32)
}

/// Makes `el` a meter named `label` reading `value` between `min` and `max`, announced as
/// `text`, or as a percentage without it.
pub fn meter(el: El, label: impl Into<String>, value: f64, min: f64, max: f64, text: Option<String>) -> El {
    let clamped = value.clamp(min.min(max), max.max(min));
    el.role("meter")
        .label(label)
        .range_value(clamped, min, max)
        .a11y_value(text.unwrap_or_else(|| percent(fraction(value, min, max))))
}

/// Makes `el` a progress bar named `label`, `value` of the way between `min` and `max`, or
/// indeterminate while `value` is `None`.
pub fn progress(el: El, label: impl Into<String>, value: Option<f64>, min: f64, max: f64, text: Option<String>) -> El {
    let el = el.role("progressbar").label(label);
    let Some(value) = value else { return el };
    let clamped = value.clamp(min.min(max), max.max(min));
    el.range_value(clamped, min, max)
        .a11y_value(text.unwrap_or_else(|| percent(fraction(value, min, max))))
}

#[cfg(test)]
mod tests {
    //! The meter and progress cases. Not ported: the label and value slots as separate elements
    //! (a Rune element carries its name and number itself), and locale number formats (metldr
    //! announces English percentages).
    use super::*;
    use rune::{Cx, Env, Runtime, col, row};

    #[test]
    fn the_fill_is_clamped_and_an_empty_range_is_empty() {
        assert_eq!(fraction(5.0, 0.0, 10.0), 0.5);
        assert_eq!(fraction(15.0, 0.0, 10.0), 1.0);
        assert_eq!(fraction(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(fraction(3.0, 4.0, 4.0), 0.0);
        assert_eq!(fraction(f64::NAN, 0.0, 1.0), 0.0);
        assert_eq!(percent(0.456), "46%");
    }

    struct App {
        done: Option<f64>,
    }

    fn view(app: &App, _: &mut Cx<App>) -> El {
        col()
            .child(meter(row().key("meter").size(100.0, 8.0), "Context used", 140.0, 0.0, 100.0, None))
            .child(progress(row().key("progress").size(100.0, 8.0), "Uploading", app.done, 0.0, 5.0, app.done.map(|d| format!("{d} of 5 files"))))
    }

    fn items(app: App) -> Vec<serde_json::Value> {
        let mut rt = Runtime::new(app, view, (200.0, 100.0), Env::new(true, 1.0));
        rt.frame(0.0);
        serde_json::from_str(&rt.a11y_json()).unwrap()
    }

    #[test]
    fn a_meter_announces_its_clamped_reading() {
        let all = items(App { done: Some(3.0) });
        let meter = all.iter().find(|i| i["role"] == "meter").unwrap();
        assert_eq!((meter["t"].as_str(), meter["vn"].as_f64(), meter["vmax"].as_f64()), (Some("Context used"), Some(100.0), Some(100.0)));
        assert_eq!(meter["vt"], "100%");
        let progress = all.iter().find(|i| i["role"] == "progressbar").unwrap();
        assert_eq!((progress["vn"].as_f64(), progress["vt"].as_str()), (Some(3.0), Some("3 of 5 files")));
    }

    #[test]
    fn an_indeterminate_progress_bar_announces_no_number() {
        let all = items(App { done: None });
        let progress = all.iter().find(|i| i["role"] == "progressbar").unwrap();
        assert!(progress.get("vn").is_none() && progress.get("vt").is_none(), "{progress}");
        assert_eq!(progress["t"], "Uploading");
    }
}
