//! The language model in compact form, about a quarter of the upstream model's memory with the
//! same answers:
//! - weights as int8, one scale per feature (the upstream f32 table is 57 MB, this 14 MB);
//! - the byte automaton as each row's differences from the start state's row: a 256-bit mask of
//!   the bytes that differ and only those targets, found by a population count (39 MB to 9 MB).
//!
//! Layout of a packed model, little endian: magic `LIDC1\0\0\0`; num_features, num_languages,
//! num_states, num_rows, num_targets, history bytes (0 for none) as u32; the start row (256 u32);
//! the masks (4 u64 per row); each row's first target (u32); the targets (u32); each state's row
//! (u16); each state's output feature (i32, -1 for none); the priors (f32 per language); the
//! scales (f32 per feature); the weights (i8, features by languages); the labels (u16 length and
//! UTF-8 bytes each).

use crate::Error;

const MAGIC: &[u8; 8] = b"LIDC1\0\0\0";

#[derive(Debug)]
pub struct Model {
    pub(crate) num_features: usize,
    pub(crate) num_languages: usize,
    pub(crate) num_states: usize,
    pub(crate) history_bytes: Option<usize>,
    start: Box<[u32; 256]>,
    masks: Vec<[u64; 4]>,
    offsets: Vec<u32>,
    targets: Vec<u32>,
    rows: Vec<u16>,
    pub(crate) outputs: Vec<i32>,
    pub(crate) priors: Vec<f32>,
    scales: Vec<f32>,
    weights: Vec<i8>,
    pub(crate) classes: Vec<String>,
}

impl Model {
    /// The state after reading `letter` in `state`.
    #[inline]
    pub(crate) fn next(&self, state: usize, letter: u8) -> usize {
        let row = usize::from(self.rows[state]);
        let mask = &self.masks[row];
        let (word, bit) = (usize::from(letter >> 6), letter & 63);
        if mask[word] >> bit & 1 == 0 {
            return self.start[usize::from(letter)] as usize;
        }
        // The target's place among the row's differing bytes: the set bits below `letter`.
        let below = mask[..word].iter().map(|m| m.count_ones()).sum::<u32>()
            + (mask[word] & ((1u64 << bit) - 1)).count_ones();
        self.targets[(self.offsets[row] + below) as usize] as usize
    }

    /// Feature `feature`'s weights for every language, and the scale that makes them floats.
    #[inline]
    pub(crate) fn weights(&self, feature: usize) -> (&[i8], f32) {
        let base = feature * self.num_languages;
        (
            &self.weights[base..base + self.num_languages],
            self.scales[feature],
        )
    }

    pub fn num_features(&self) -> usize {
        self.num_features
    }

    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// A packed model; an error when the bytes are not one.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let bad = |what: &str| Error::InvalidModel(format!("packed model: {what}"));
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], Error> {
            let slice = bytes.get(at..at + n).ok_or_else(|| bad("truncated"))?;
            at += n;
            Ok(slice)
        };
        if take(8)? != MAGIC {
            return Err(bad("magic"));
        }
        let mut header = [0usize; 6];
        for value in &mut header {
            *value = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        }
        let [
            num_features,
            num_languages,
            num_states,
            num_rows,
            num_targets,
            history,
        ] = header;
        let u32s = |b: &[u8]| -> Vec<u32> {
            b.chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        };
        let f32s = |b: &[u8]| -> Vec<f32> {
            b.chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        };
        let start: Box<[u32; 256]> = u32s(take(256 * 4)?)
            .into_boxed_slice()
            .try_into()
            .map_err(|_| bad("start row"))?;
        let masks = take(num_rows * 32)?
            .chunks_exact(32)
            .map(|c| {
                std::array::from_fn(|i| u64::from_le_bytes(c[i * 8..i * 8 + 8].try_into().unwrap()))
            })
            .collect();
        let offsets = u32s(take(num_rows * 4)?);
        let targets = u32s(take(num_targets * 4)?);
        let rows: Vec<u16> = take(num_states * 2)?
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let outputs = take(num_states * 4)?
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let priors = f32s(take(num_languages * 4)?);
        let scales = f32s(take(num_features * 4)?);
        let weights = take(num_features * num_languages)?
            .iter()
            .map(|&b| b as i8)
            .collect();
        let mut classes = Vec::with_capacity(num_languages);
        for _ in 0..num_languages {
            let length = usize::from(u16::from_le_bytes(take(2)?.try_into().unwrap()));
            classes.push(String::from_utf8(take(length)?.to_vec()).map_err(|_| bad("label"))?);
        }
        if at != bytes.len() {
            return Err(bad("trailing bytes"));
        }
        let model = Self {
            num_features,
            num_languages,
            num_states,
            history_bytes: (history > 0).then_some(history),
            start,
            masks,
            offsets,
            targets,
            rows,
            outputs,
            priors,
            scales,
            weights,
            classes,
        };
        model.check().map_err(|what| bad(what))?;
        Ok(model)
    }

    /// Every index the lookups use is in range.
    fn check(&self) -> Result<(), &'static str> {
        let states = self.num_states as u32;
        if self.start.iter().chain(&self.targets).any(|&s| s >= states) {
            return Err("state out of range");
        }
        if self
            .rows
            .iter()
            .any(|&r| usize::from(r) >= self.masks.len())
        {
            return Err("row out of range");
        }
        for (mask, &offset) in self.masks.iter().zip(&self.offsets) {
            let count: u32 = mask.iter().map(|m| m.count_ones()).sum();
            if (offset + count) as usize > self.targets.len() {
                return Err("targets out of range");
            }
        }
        if self
            .outputs
            .iter()
            .any(|&f| f < -1 || f >= self.num_features as i32)
        {
            return Err("feature out of range");
        }
        if self.outputs.len() != self.num_states
            || self
                .priors
                .iter()
                .chain(&self.scales)
                .any(|v| !v.is_finite())
        {
            return Err("tables");
        }
        Ok(())
    }

    /// Packs an upstream LIDG2 model (gzip, f32) into the compact form, with `history` bytes of
    /// lookbehind for the parallel feature scan.
    #[cfg(feature = "pack")]
    pub fn pack_original(lidg: &[u8], history: Option<usize>) -> Result<Vec<u8>, Error> {
        let original = crate::original::Original::from_bytes(lidg)?;
        let num_rows = original.transitions.len() / 256;
        if num_rows > usize::from(u16::MAX) + 1 {
            return Err(Error::InvalidModel("too many rows for u16".into()));
        }
        let start_row = original.rows[0] as usize;
        let start = &original.transitions[start_row * 256..start_row * 256 + 256];
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        let mut masks = Vec::with_capacity(num_rows * 32);
        let (mut offsets, mut targets) = (Vec::with_capacity(num_rows), Vec::new());
        for row in original.transitions.chunks_exact(256) {
            let mut mask = [0u64; 4];
            offsets.push(targets.len() as u32);
            for (letter, (&target, &fallback)) in row.iter().zip(start).enumerate() {
                if target != fallback {
                    mask[letter >> 6] |= 1 << (letter & 63);
                    targets.push(target);
                }
            }
            masks.extend(mask.iter().flat_map(|m| m.to_le_bytes()));
        }
        let header = [
            original.num_features,
            original.num_languages,
            original.num_states,
            num_rows,
            targets.len(),
            history.unwrap_or(0),
        ];
        for value in header {
            out.extend_from_slice(&(value as u32).to_le_bytes());
        }
        start
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        out.extend_from_slice(&masks);
        offsets
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        targets
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        original
            .rows
            .iter()
            .for_each(|&v| out.extend_from_slice(&(v as u16).to_le_bytes()));
        original
            .outputs
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        original
            .priors
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        let rows = original.weights.chunks_exact(original.num_languages);
        let mut weights = Vec::with_capacity(original.weights.len());
        for row in rows {
            let scale = row.iter().fold(0.0f32, |m, v| m.max(v.abs())) / 127.0;
            out.extend_from_slice(&scale.to_le_bytes());
            let inverse = if scale > 0.0 { 1.0 / scale } else { 0.0 };
            weights.extend(
                row.iter()
                    .map(|v| (v * inverse).round().clamp(-127.0, 127.0) as i8 as u8),
            );
        }
        out.extend_from_slice(&weights);
        for label in &original.classes {
            out.extend_from_slice(&(label.len() as u16).to_le_bytes());
            out.extend_from_slice(label.as_bytes());
        }
        Ok(out)
    }
}
