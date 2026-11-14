use crate::model::RunRoute;

pub(crate) fn route(route: &RunRoute) -> String {
    match route {
        RunRoute::LegacyUnspecified => "legacy route unspecified".to_string(),
        RunRoute::Fixture => "fixture".to_string(),
        RunRoute::Messages { model } => format!("Owned model fixture, model {model}"),
        RunRoute::HttpsMessages { model, .. } => format!("Configured HTTPS model, model {model}"),
        RunRoute::HttpsChatCompletions { endpoint, model, .. } => format!("Chat Completions, model {model}, HTTPS endpoint {endpoint}"),
        RunRoute::HttpsResponses { endpoint, model, .. } => format!("OpenAI Responses, model {model}, HTTPS endpoint {endpoint}"),
        RunRoute::HttpsGoogleInteractions { endpoint, model, .. } => format!("Google Interactions, model {model}, HTTPS endpoint {endpoint}"),
        RunRoute::Text { protocol, endpoint, model, .. } => format!("{} page questions, model {model}, HTTPS endpoint {endpoint}. Sends the saved page and completed answers from this page and provider selection.", match protocol {
            crate::model::ProviderProtocol::Messages => "Messages",
            crate::model::ProviderProtocol::Responses => "OpenAI Responses",
            crate::model::ProviderProtocol::GoogleInteractions => "Google Interactions",
            crate::model::ProviderProtocol::ChatCompletions => "Chat Completions",
        }),
        RunRoute::Pi {
            provider, model, ..
        } => format!(
            "Pi configured provider {provider}, model {model}. Pi manages its endpoint, account, tools and permissions."
        ),
        RunRoute::Codex { provider, model, .. } => match (provider, model) {
            (Some(provider), Some(model)) => format!("Codex configured provider {provider}, model {model}. Codex manages its endpoint, account, tools and permissions."),
            _ => "Codex native session. Codex manages its provider, endpoint, account, tools and permissions.".to_string(),
        },
        RunRoute::Harness { harness, .. } if harness == "claude" => {
            "Claude Code native session. Claude Code manages its model, account, tools and permissions.".to_string()
        }
        RunRoute::Harness { .. } => "Native agent session.".to_string(),
        RunRoute::OpenCode { model, mode, .. } => format!(
            "OpenCode native session{}{}.",
            model.as_ref().map(|value| format!(" using model {value}")).unwrap_or_default(),
            mode.as_ref().map(|value| format!(" in {value} mode")).unwrap_or_default(),
        ),
    }
}

/// Compact count: 950, 1.25K, 12.5K, 125K, 1.25M.
pub(crate) fn tokens(value: u64) -> String {
    let scaled = [(1e12, 'T'), (1e9, 'B'), (1e6, 'M'), (1e3, 'K')]
        .into_iter()
        .find(|(unit, _)| value as f64 >= *unit);
    let Some((unit, suffix)) = scaled else {
        return value.to_string();
    };
    let value = value as f64 / unit;
    let digits = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{text}{suffix}")
}

/// 850 ms, 4.2 s, 3 min 12 s, 1 h 05 min.
pub(crate) fn duration(ms: u32) -> String {
    let seconds = ms / 1000;
    match ms {
        0..1_000 => format!("{ms} ms"),
        1_000..60_000 => format!("{:.1} s", f64::from(ms) / 1000.0),
        60_000..3_600_000 => format!("{} min {} s", seconds / 60, seconds % 60),
        _ => format!("{} h {:02} min", seconds / 3600, seconds / 60 % 60),
    }
}

/// USD in cents: $0.04, and <$0.01 for a smaller nonzero cost.
pub(crate) fn usd(value: f64) -> String {
    if value > 0.0 && value < 0.005 {
        "<$0.01".into()
    } else {
        format!("${value:.2}")
    }
}

/// The cost of a run on an API route: priced, local, or unpriced.
fn cost(route: &RunRoute, usage: metldr_conversation::Usage) -> Option<String> {
    let (endpoint, model) = match route {
        RunRoute::HttpsMessages { endpoint, model, .. }
        | RunRoute::HttpsChatCompletions { endpoint, model, .. }
        | RunRoute::HttpsResponses { endpoint, model, .. }
        | RunRoute::HttpsGoogleInteractions { endpoint, model, .. }
        | RunRoute::Text { endpoint, model, .. } => (endpoint, model),
        _ => return None,
    };
    if endpoint.starts_with("http://") {
        return Some("local".into());
    }
    Some(match metldr_conversation::price(model) {
        Some(price) => usd(usage.cost(price)?),
        None => "Unpriced".into(),
    })
}

/// One line of tokens, timings and cost for run details; unknown counts are left out.
pub(crate) fn ledger(ledger: &metldr_conversation::Ledger, route: &RunRoute) -> String {
    let total = ledger.total();
    let mut parts = Vec::with_capacity(5);
    if let Some(input) = total.input {
        let mut part = format!("{} in", tokens(input));
        if let Some(cached) = total.cached.filter(|_| input > 0) {
            let share = cached as f64 * 100.0 / input as f64;
            part.push_str(&format!(", {} cached ({share:.1}%)", tokens(cached)));
        }
        parts.push(part);
    }
    if let Some(output) = total.output {
        parts.push(format!("{} out", tokens(output)));
    }
    let calls = ledger.steps.iter().filter(|step| step.call).count();
    let steps = ledger.steps.len();
    parts.push(match calls {
        0 => format!("{steps} {}", if steps == 1 { "step" } else { "steps" }),
        _ => format!(
            "{steps} steps, {calls} tool {}",
            if calls == 1 { "call" } else { "calls" }
        ),
    });
    if let Some(first) = ledger.steps.first().and_then(|step| step.first_ms) {
        parts.push(format!("first token {}", duration(first)));
    }
    parts.push(duration(ledger.ms));
    parts.extend(cost(route, total));
    parts.join(" · ")
}

#[cfg(test)]
mod ledger_checks {
    use metldr_conversation::{Ledger, Step, Usage};

    #[test]
    fn formats_counts_and_ledger() {
        let cases = [
            (950, "950"),
            (1_250, "1.25K"),
            (12_500, "12.5K"),
            (125_000, "125K"),
            (1_000_000, "1M"),
        ];
        for (value, text) in cases {
            assert_eq!(super::tokens(value), text);
        }
        let usage = |input, cached, output| Usage {
            input: Some(input),
            cached: Some(cached),
            written: None,
            output: Some(output),
            reasoning: None,
        };
        let ledger = Ledger {
            steps: vec![
                Step {
                    usage: usage(6_000, 0, 200),
                    first_ms: Some(850),
                    ms: 1_900,
                    call: true,
                },
                Step {
                    usage: usage(6_500, 6_000, 1_000),
                    first_ms: Some(400),
                    ms: 2_000,
                    call: false,
                },
            ],
            ms: 4_200,
        };
        let route = |endpoint: &str, model: &str| crate::model::RunRoute::HttpsResponses {
            endpoint: endpoint.into(),
            model: model.into(),
            revision: 1,
            fingerprint: String::new(),
        };
        let line = "12.5K in, 6K cached (48.0%) · 1.2K out · 2 steps, 1 tool call · first token 850 ms · 4.2 s";
        let gpt = route("https://api.openai.com/v1/responses", "gpt-5");
        assert_eq!(super::ledger(&ledger, &gpt), format!("{line} · $0.02"));
        let local = route("http://127.0.0.1:11434/v1/responses", "llama3");
        assert_eq!(super::ledger(&ledger, &local), format!("{line} · local"));
        let unknown = route("https://example.com/v1/responses", "house-model");
        assert_eq!(super::ledger(&ledger, &unknown), format!("{line} · Unpriced"));
        assert_eq!(super::usd(0.004), "<$0.01");
        assert_eq!(super::duration(192_000), "3 min 12 s");
        assert_eq!(super::duration(3_900_000), "1 h 05 min");
    }
}
