//! Built-in model prices in USD per million tokens, as of 14 November 2026, for the providers
//! metldr connects to.

use crate::Usage;
use std::sync::OnceLock;

pub const PRICES_AS_OF: &str = "14 November 2026";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cached: Option<f64>,
    pub written: Option<f64>,
    pub output: f64,
}

fn table() -> &'static [(&'static str, Price)] {
    static TABLE: OnceLock<Vec<(&'static str, Price)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../assets/prices.tsv")
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let model = fields.next()?;
                let mut rate = || fields.next().and_then(|field| field.parse().ok());
                let (input, cached, written, output) = (rate(), rate(), rate(), rate());
                Some((model, Price { input: input?, cached, written, output: output? }))
            })
            .collect()
    })
}

/// The price of a model id as a provider reports it: exact, then without a vendor prefix
/// (`anthropic/claude-sonnet-4.5`), then with dots as dashes (`claude-sonnet-4-5`).
pub fn price(model: &str) -> Option<Price> {
    let key = model.trim().to_ascii_lowercase();
    let bare = key.rsplit('/').next().unwrap_or(&key);
    let dashed = bare.replace('.', "-");
    let table = table();
    [key.as_str(), bare, dashed.as_str()].into_iter().find_map(|candidate| {
        table
            .binary_search_by(|(model, _)| (*model).cmp(candidate))
            .ok()
            .map(|at| table[at].1)
    })
}

impl Usage {
    /// Cost in USD, or `None` while input or output is unknown.
    pub fn cost(&self, price: Price) -> Option<f64> {
        let (input, output) = (self.input?, self.output?);
        let cached = self.cached.unwrap_or(0);
        let written = self.written.unwrap_or(0);
        let fresh = input.saturating_sub(cached).saturating_sub(written);
        let millionths = fresh as f64 * price.input
            + cached as f64 * price.cached.unwrap_or(price.input)
            + written as f64 * price.written.unwrap_or(price.input)
            + output as f64 * price.output;
        Some(millionths / 1e6)
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn looks_up_provider_ids_and_prices_cache_reads_and_writes() {
        let sonnet = price("anthropic/claude-sonnet-4.5").unwrap();
        assert_eq!(price("claude-sonnet-4-5"), Some(sonnet));
        assert_eq!((sonnet.input, sonnet.cached, sonnet.written, sonnet.output), (3.0, Some(0.3), Some(3.75), 15.0));
        assert!(price("gpt-5").is_some() && price("no-such-model").is_none());
        let usage = Usage { input: Some(1_000_000), cached: Some(600_000), written: Some(100_000), output: Some(10_000), reasoning: None };
        let cost = usage.cost(sonnet).unwrap();
        assert!((cost - (0.3 * 3.0 + 0.6 * 0.3 + 0.1 * 3.75 + 0.01 * 15.0)).abs() < 1e-9);
        assert!(table().windows(2).all(|pair| pair[0].0 < pair[1].0));
    }
}
