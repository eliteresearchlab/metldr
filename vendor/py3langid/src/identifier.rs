use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::{preprocess, Error, Model};

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub normalized: bool,
    pub min_confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LanguageScore {
    pub language: String,
    pub score: f64,
}

pub struct Identifier {
    model: Arc<Model>,
    runtime: RwLock<Arc<Runtime>>,
    options: Options,
}

struct Runtime {
    columns: Vec<usize>,
    label_indices: Vec<usize>,
    alias_pairs: Vec<(usize, usize)>,
    pool: Mutex<Vec<WorkBuffer>>,
    max_cached_buffers: usize,
}

struct WorkBuffer {
    feature_counts: Vec<u32>,
    active_features: Vec<usize>,
    scores: Vec<f32>,
}

impl Identifier {
    /// The identifier for a packed model (see `Model::from_bytes`).
    pub fn from_bytes(bytes: &[u8], options: Options) -> Result<Self, Error> {
        Self::from_model(Arc::new(Model::from_bytes(bytes)?), options)
    }

    pub fn from_model(model: Arc<Model>, options: Options) -> Result<Self, Error> {
        if let Some(threshold) = options.min_confidence {
            if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                return Err(Error::InvalidConfiguration(
                    "minimum confidence must be finite and between 0 and 1".into(),
                ));
            }
            if !options.normalized {
                return Err(Error::InvalidConfiguration(
                    "minimum confidence requires normalized probabilities".into(),
                ));
            }
        }
        let runtime = Arc::new(Runtime::new(&model, (0..model.num_languages).collect()));
        Ok(Self {
            model,
            runtime: RwLock::new(runtime),
            options,
        })
    }

    pub fn classes(&self) -> Vec<String> {
        let runtime = self.snapshot();
        runtime
            .label_indices
            .iter()
            .map(|&index| self.model.classes[runtime.columns[index]].clone())
            .collect()
    }

    pub fn set_languages(&self, languages: &[&str]) -> Result<(), Error> {
        if languages.is_empty() {
            self.reset_languages();
            return Ok(());
        }
        for &language in languages {
            if !self.model.classes.iter().any(|label| label == language) {
                return Err(Error::InvalidConfiguration(format!(
                    "unknown language: {language}"
                )));
            }
        }
        let columns = self
            .model
            .classes
            .iter()
            .enumerate()
            .filter_map(|(index, label)| languages.contains(&label.as_str()).then_some(index))
            .collect();
        let runtime = Arc::new(Runtime::new(&self.model, columns));
        *self
            .runtime
            .write()
            .unwrap_or_else(|error| error.into_inner()) = runtime;
        Ok(())
    }

    pub fn reset_languages(&self) {
        let runtime = Arc::new(Runtime::new(
            &self.model,
            (0..self.model.num_languages).collect(),
        ));
        *self
            .runtime
            .write()
            .unwrap_or_else(|error| error.into_inner()) = runtime;
    }

    pub fn identify(&self, text: impl AsRef<[u8]>) -> LanguageScore {
        self.identify_mode(text.as_ref(), self.options.normalized)
    }

    pub fn identify_normalized(&self, text: impl AsRef<[u8]>) -> LanguageScore {
        self.identify_mode(text.as_ref(), true)
    }

    pub fn rank(&self, text: impl AsRef<[u8]>) -> Vec<LanguageScore> {
        self.rank_mode(text.as_ref(), self.options.normalized)
    }

    pub fn rank_normalized(&self, text: impl AsRef<[u8]>) -> Vec<LanguageScore> {
        self.rank_mode(text.as_ref(), true)
    }

    fn snapshot(&self) -> Arc<Runtime> {
        Arc::clone(
            &self
                .runtime
                .read()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }

    fn identify_mode(&self, text: &[u8], normalized: bool) -> LanguageScore {
        self.with_scores(text, normalized, |runtime, scores| {
            let mut best = runtime.label_indices[0];
            for &index in &runtime.label_indices[1..] {
                if scores[index] > scores[best] {
                    best = index;
                }
            }
            let score = f64::from(scores[best]);
            let language = if normalized
                && self
                    .options
                    .min_confidence
                    .is_some_and(|threshold| score < threshold)
            {
                "und".into()
            } else {
                self.model.classes[runtime.columns[best]].clone()
            };
            LanguageScore { language, score }
        })
    }

    fn rank_mode(&self, text: &[u8], normalized: bool) -> Vec<LanguageScore> {
        self.with_scores(text, normalized, |runtime, scores| {
            let mut results: Vec<_> = runtime
                .label_indices
                .iter()
                .map(|&index| LanguageScore {
                    language: self.model.classes[runtime.columns[index]].clone(),
                    score: f64::from(scores[index]),
                })
                .collect();
            results.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(Ordering::Equal)
            });
            results
        })
    }

    fn with_scores<Value>(
        &self,
        text: &[u8],
        normalized: bool,
        consume: impl FnOnce(&Runtime, &[f32]) -> Value,
    ) -> Value {
        let runtime = self.snapshot();
        let mut buffer = runtime
            .pool
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop()
            .unwrap_or_else(|| WorkBuffer {
                feature_counts: vec![0; self.model.num_features],
                active_features: Vec::new(),
                scores: vec![0.0; runtime.columns.len()],
            });
        self.score(&runtime, &mut buffer, text, normalized);
        let result = consume(&runtime, &buffer.scores);
        let mut pool = runtime
            .pool
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pool.len() < runtime.max_cached_buffers {
            pool.push(buffer);
        }
        result
    }

    fn extract_features<const LANES: usize>(&self, buffer: &mut WorkBuffer, text: &[u8]) {
        let mut state = 0;
        let mut remaining = text;
        if let Some(history) = self.model.history_bytes {
            let mut emitted = [[0_i32; 256]; LANES];
            while remaining.len() >= LANES * 16 {
                let width = remaining.len().min(LANES * 256) / LANES;
                if width < history {
                    break;
                }
                let mut states = [0; LANES];
                states[0] = state;
                for (lane, state) in states.iter_mut().enumerate().skip(1) {
                    let start = lane * width;
                    for &letter in &remaining[start - history..start] {
                        *state = self.model.next(*state, letter);
                    }
                }
                for offset in 0..width {
                    for (lane, state) in states.iter_mut().enumerate() {
                        let letter = remaining[lane * width + offset];
                        *state = self.model.next(*state, letter);
                        emitted[lane][offset] = self.model.outputs[*state];
                    }
                }
                for lane in &emitted {
                    for &feature in &lane[..width] {
                        if feature >= 0 {
                            buffer.record_feature(feature as usize);
                        }
                    }
                }
                state = states[LANES - 1];
                remaining = &remaining[width * LANES..];
            }
        }
        for &letter in remaining {
            state = self.model.next(state, letter);
            let feature = self.model.outputs[state];
            if feature >= 0 {
                buffer.record_feature(feature as usize);
            }
        }
    }

    fn score(&self, runtime: &Runtime, buffer: &mut WorkBuffer, text: &[u8], normalized: bool) {
        let text = preprocess::encode(text);
        if text.len() >= 128 {
            self.extract_features::<8>(buffer, text.as_ref());
        } else {
            self.extract_features::<4>(buffer, text.as_ref());
        }
        buffer.scores.fill(0.0);
        if buffer.active_features.is_empty() {
            if !normalized {
                buffer.scores.fill(-f32::MAX);
            }
        } else {
            let mut remaining_features = buffer.active_features.as_slice();
            if runtime.columns.len() == self.model.num_languages {
                let (groups, tail) = buffer.active_features.as_chunks::<4>();
                remaining_features = tail;
                for features in groups {
                    // Each feature's weights as int8 times its scale: the scale folds into its count.
                    let mut counts = [0.0f32; 4];
                    let weights: [&[i8]; 4] = std::array::from_fn(|index| {
                        let feature = features[index];
                        let (row, scale) = self.model.weights(feature);
                        counts[index] = f64::from(buffer.feature_counts[feature]).ln_1p() as f32 * scale;
                        buffer.feature_counts[feature] = 0;
                        row
                    });
                    for ((((score, &first), &second), &third), &fourth) in buffer
                        .scores
                        .iter_mut()
                        .zip(weights[0])
                        .zip(weights[1])
                        .zip(weights[2])
                        .zip(weights[3])
                    {
                        *score += counts[0] * f32::from(first);
                        *score += counts[1] * f32::from(second);
                        *score += counts[2] * f32::from(third);
                        *score += counts[3] * f32::from(fourth);
                    }
                }
            }
            for &feature in remaining_features {
                let (weights, scale) = self.model.weights(feature);
                let count = f64::from(buffer.feature_counts[feature]).ln_1p() as f32 * scale;
                buffer.feature_counts[feature] = 0;
                if runtime.columns.len() == self.model.num_languages {
                    for (score, &weight) in buffer.scores.iter_mut().zip(weights) {
                        *score += count * f32::from(weight);
                    }
                } else {
                    for (score, &column) in buffer.scores.iter_mut().zip(&runtime.columns) {
                        *score += count * f32::from(weights[column]);
                    }
                }
            }
            for (score, &column) in buffer.scores.iter_mut().zip(&runtime.columns) {
                *score += self.model.priors[column];
            }
        }
        buffer.active_features.clear();
        if normalized {
            let scale = (1.0 / (text.len().max(1) as f64).sqrt()) as f32;
            let mut maximum = -f32::MAX;
            for score in &mut buffer.scores {
                *score *= scale;
                maximum = maximum.max(*score);
            }
            let mut total = 0.0_f32;
            for score in &mut buffer.scores {
                *score = f64::from(*score - maximum).exp() as f32;
                total += *score;
            }
            for score in &mut buffer.scores {
                *score /= total;
            }
        }
        for &(first, alias) in &runtime.alias_pairs {
            if normalized {
                buffer.scores[first] += buffer.scores[alias];
                buffer.scores[alias] = 0.0;
            } else {
                buffer.scores[first] = buffer.scores[first].max(buffer.scores[alias]);
                buffer.scores[alias] = -f32::MAX;
            }
        }
    }
}

impl Runtime {
    fn new(model: &Model, columns: Vec<usize>) -> Self {
        let mut first = HashMap::new();
        let mut label_indices = Vec::new();
        let mut alias_pairs = Vec::new();
        for (index, &column) in columns.iter().enumerate() {
            let label = &model.classes[column];
            if let Some(&previous) = first.get(label) {
                alias_pairs.push((previous, index));
            } else {
                first.insert(label, index);
                label_indices.push(index);
            }
        }
        Self {
            columns,
            label_indices,
            alias_pairs,
            pool: Mutex::new(Vec::new()),
            max_cached_buffers: std::thread::available_parallelism().map_or(1, |count| count.get()),
        }
    }
}

impl WorkBuffer {
    fn record_feature(&mut self, feature: usize) {
        if self.feature_counts[feature] == 0 {
            self.active_features.push(feature);
        }
        self.feature_counts[feature] = self.feature_counts[feature].wrapping_add(1);
    }
}

