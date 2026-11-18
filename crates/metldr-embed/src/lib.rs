//! Static embeddings for search: each token has a stored vector, and a text's embedding is the
//! normalized mean of its tokens' vectors. No neural network runs; a text embeds in microseconds.
//!
//! The model is one packed file (see [`pack`]): the vocabulary, one scale per token and each
//! token's vector as int8. Chunk vectors are kept as int8 with one scale too, and compared to
//! the query by integer dot products.

mod wordpiece;

use wordpiece::WordPiece;

const MAGIC: &[u8; 8] = b"MTLDRE01";

pub struct Model {
    bytes: Vec<u8>,
    dim: usize,
    scales: usize,
    rows: usize,
    tokens: usize,
    tokenizer: WordPiece,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
}

impl Model {
    /// The model in a packed file; None when the file is not one.
    pub fn parse(bytes: Vec<u8>) -> Option<Self> {
        if bytes.get(..8)? != MAGIC {
            return None;
        }
        let (dim, tokens, unk, vocab_len) = (
            u32_at(&bytes, 8)?,
            u32_at(&bytes, 12)?,
            u32_at(&bytes, 16)?,
            u32_at(&bytes, 20)?,
        );
        let vocab_end = 24 + vocab_len;
        let scales = vocab_end;
        let rows = scales + tokens * 4;
        if bytes.len() != rows + tokens * dim {
            return None;
        }
        let vocab = std::str::from_utf8(&bytes[24..vocab_end]).ok()?;
        let tokenizer = WordPiece::new(vocab, unk as u32);
        Some(Self {
            bytes,
            dim,
            scales,
            rows,
            tokens,
            tokenizer,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The normalized embedding of `text` into `out` (`dim` long); all zeros when no token is
    /// known. `ids` is scratch, kept between calls to save allocations.
    pub fn embed(&self, text: &str, ids: &mut Vec<u32>, out: &mut [f32]) {
        self.tokenizer.encode(text, ids);
        out.fill(0.0);
        let unk = self.tokenizer.unk();
        for &id in ids
            .iter()
            .filter(|&&id| id != unk && (id as usize) < self.tokens)
        {
            let id = id as usize;
            let scale =
                f32::from_le_bytes(self.bytes[self.scales + id * 4..][..4].try_into().unwrap());
            let row = &self.bytes[self.rows + id * self.dim..][..self.dim];
            for (o, &v) in out.iter_mut().zip(row) {
                *o += f32::from(v as i8) * scale;
            }
        }
        let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            out.iter_mut().for_each(|v| *v /= norm);
        }
    }
}

/// `vector` as int8 times one scale, the form chunks and queries are compared in.
pub fn quantize(vector: &[f32], out: &mut Vec<i8>) -> f32 {
    let scale = vector.iter().fold(0.0f32, |m, v| m.max(v.abs())) / 127.0;
    out.clear();
    if scale == 0.0 {
        out.resize(vector.len(), 0);
        return 0.0;
    }
    out.extend(vector.iter().map(|v| (v / scale).round() as i8));
    scale
}

/// The integer dot product of two int8 vectors. With wasm SIMD, 16 bytes at a time: one load
/// per vector, widened to i16 and multiplied and added pairwise into four i32 lanes
/// (`i32x4.dot_i16x8_s`).
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[allow(unsafe_code)]
pub fn dot(a: &[i8], b: &[i8]) -> i32 {
    use core::arch::wasm32::*;
    let mut sum = i32x4_splat(0);
    let (a_chunks, b_chunks) = (a.chunks_exact(16), b.chunks_exact(16));
    let tail: i32 = a_chunks
        .remainder()
        .iter()
        .zip(b_chunks.remainder())
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum();
    for (x, y) in a_chunks.zip(b_chunks) {
        // SAFETY: `chunks_exact(16)` yields exactly 16 initialized bytes each, and `v128_load`
        // has no alignment requirement, so each load reads only within its chunk.
        let (x, y) = unsafe { (v128_load(x.as_ptr().cast()), v128_load(y.as_ptr().cast())) };
        sum = i32x4_add(
            sum,
            i32x4_dot_i16x8(i16x8_extend_low_i8x16(x), i16x8_extend_low_i8x16(y)),
        );
        sum = i32x4_add(
            sum,
            i32x4_dot_i16x8(i16x8_extend_high_i8x16(x), i16x8_extend_high_i8x16(y)),
        );
    }
    i32x4_extract_lane::<0>(sum)
        + i32x4_extract_lane::<1>(sum)
        + i32x4_extract_lane::<2>(sum)
        + i32x4_extract_lane::<3>(sum)
        + tail
}

/// The integer dot product of two int8 vectors, in independent lanes so it vectorizes.
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
pub fn dot(a: &[i8], b: &[i8]) -> i32 {
    let mut lanes = [0i32; 16];
    let (a_chunks, b_chunks) = (a.chunks_exact(16), b.chunks_exact(16));
    let tail: i32 = a_chunks
        .remainder()
        .iter()
        .zip(b_chunks.remainder())
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum();
    for (x, y) in a_chunks.zip(b_chunks) {
        for i in 0..16 {
            lanes[i] += i32::from(x[i]) * i32::from(y[i]);
        }
    }
    lanes.iter().sum::<i32>() + tail
}

/// The packed file for a Model2Vec model: `tokens` in id order, the unknown token's id, and
/// the float vectors (`tokens.len()` rows of `dim`), each row stored as int8 times its scale.
pub fn pack(tokens: &[&str], unk: u32, dim: usize, vectors: &[f32]) -> Vec<u8> {
    let vocab = tokens.join("\n");
    let mut out = Vec::with_capacity(24 + vocab.len() + tokens.len() * (4 + dim));
    out.extend_from_slice(MAGIC);
    for n in [dim, tokens.len(), unk as usize, vocab.len()] {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
    out.extend_from_slice(vocab.as_bytes());
    let mut rows = Vec::with_capacity(tokens.len() * dim);
    let mut row = Vec::new();
    for vector in vectors.chunks_exact(dim) {
        let scale = quantize(vector, &mut row);
        out.extend_from_slice(&scale.to_le_bytes());
        rows.extend(row.iter().map(|&v| v as u8));
    }
    out.extend_from_slice(&rows);
    out
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn a_packed_model_embeds_known_tokens_and_skips_unknown_ones() {
        let tokens = ["[UNK]", "cat", "##s", "dog", "."];
        let vectors = [0.0, 0.0, 1.0, 0.0, 0.5, 0.5, 0.0, 1.0, 0.0, 0.0];
        let model = Model::parse(pack(&tokens, 0, 2, &vectors)).unwrap();
        let (mut ids, mut out) = (Vec::new(), [0.0f32; 2]);
        model.embed("Cats, dog!", &mut ids, &mut out);
        // "cat"+"##s"+"dog" = (1.5, 1.5) normalized; "," and "!" are unknown.
        assert!((out[0] - out[1]).abs() < 1e-3 && (out[0] - 0.7071).abs() < 2e-3);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        quantize(&[1.0, -0.5, 0.25], &mut a);
        quantize(&[1.0, 1.0, 1.0], &mut b);
        assert_eq!(dot(&a, &b), 127 * 127 - 64 * 127 + 32 * 127);
        assert!(Model::parse(b"not a model".to_vec()).is_none());
    }
}
