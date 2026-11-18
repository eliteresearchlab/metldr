//! The upstream LIDG2 model reader (gzip, f32 weights, full transition table), kept only to
//! convert a model to the compact form in `model.rs` (feature `pack`).

use std::io::{BufReader, Read};

use flate2::read::MultiGzDecoder;

use crate::Error;

const MAGIC: &[u8; 6] = b"LIDG2\0";
const DIMENSION_LIMITS: [u32; 4] = [1_000_000, 10_000, 500_000, 500_000];
const MAX_WEIGHTS: usize = 50_000_000;

#[derive(Debug)]
pub(crate) struct Original {
    pub(crate) num_features: usize,
    pub(crate) num_languages: usize,
    pub(crate) num_states: usize,
    pub(crate) history_bytes: Option<usize>,
    pub(crate) transitions: Vec<u32>,
    pub(crate) rows: Vec<u32>,
    pub(crate) outputs: Vec<i32>,
    pub(crate) priors: Vec<f32>,
    pub(crate) weights: Vec<f32>,
    pub(crate) classes: Vec<String>,
}

impl Original {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Self::from_reader(bytes)
    }

    fn from_reader(mut source: impl Read) -> Result<Self, Error> {
        let mut signature = [0; 6];
        source.read_exact(&mut signature)?;
        if &signature != MAGIC {
            return Err(Error::InvalidModel("expected LIDG2 signature".into()));
        }
        let mut reader = BufReader::new(MultiGzDecoder::new(source));
        let dimensions = read_array(&mut reader, 4, u32::from_le_bytes)?;
        for (index, (&value, &limit)) in dimensions.iter().zip(&DIMENSION_LIMITS).enumerate() {
            if value == 0 || value > limit {
                return Err(Error::InvalidModel(format!(
                    "dimension {index}: {value} (limit {limit})"
                )));
            }
        }
        if dimensions[3] > dimensions[2] {
            return Err(Error::InvalidModel(
                "more transition rows than states".into(),
            ));
        }
        let num_features = dimensions[0] as usize;
        let num_languages = dimensions[1] as usize;
        let num_states = dimensions[2] as usize;
        let num_rows = dimensions[3] as usize;
        let num_weights = num_features
            .checked_mul(num_languages)
            .filter(|&count| count <= MAX_WEIGHTS)
            .ok_or_else(|| Error::InvalidModel("likelihood dimensions exceed limit".into()))?;
        let transitions = read_array(&mut reader, num_rows * 256, u32::from_le_bytes)?;
        let rows = read_array(&mut reader, num_states, u32::from_le_bytes)?;
        let outputs = read_array(&mut reader, num_states, i32::from_le_bytes)?;
        let priors = read_array(&mut reader, num_languages, f32::from_le_bytes)?;
        let weights = read_array(&mut reader, num_weights, f32::from_le_bytes)?;
        let mut classes = Vec::with_capacity(num_languages);
        for _ in 0..num_languages {
            let mut length = [0; 2];
            reader.read_exact(&mut length)?;
            let length = usize::from(u16::from_le_bytes(length));
            if length == 0 || length > 256 {
                return Err(Error::InvalidModel(format!("class label length: {length}")));
            }
            let mut label = vec![0; length];
            reader.read_exact(&mut label)?;
            classes.push(
                String::from_utf8(label)
                    .map_err(|_| Error::InvalidModel("class label is not UTF-8".into()))?,
            );
        }
        let mut extra = [0; 1];
        if reader.read(&mut extra)? != 0 {
            return Err(Error::InvalidModel("unexpected trailing model data".into()));
        }
        for (index, &state) in transitions.iter().enumerate() {
            if state as usize >= num_states {
                return Err(Error::InvalidModel(format!(
                    "transition {index}: state {state}"
                )));
            }
        }
        for (state, (&row, &feature)) in rows.iter().zip(&outputs).enumerate() {
            if row as usize >= num_rows {
                return Err(Error::InvalidModel(format!("row {row} for state {state}")));
            }
            if feature < -1 || feature >= num_features as i32 {
                return Err(Error::InvalidModel(format!(
                    "output feature {feature} for state {state}"
                )));
            }
        }
        if priors
            .iter()
            .chain(&weights)
            .any(|score| !score.is_finite())
        {
            return Err(Error::InvalidModel("non-finite score".into()));
        }
        Ok(Self {
            num_features,
            num_languages,
            num_states,
            history_bytes: None,
            transitions,
            rows,
            outputs,
            priors,
            weights,
            classes,
        })
    }
}

fn read_array<Value>(
    reader: &mut impl Read,
    count: usize,
    decode: impl Fn([u8; 4]) -> Value,
) -> Result<Vec<Value>, Error> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|error| Error::InvalidModel(format!("array allocation: {error}")))?;
    for _ in 0..count {
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes)?;
        values.push(decode(bytes));
    }
    Ok(values)
}

