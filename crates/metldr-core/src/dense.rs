//! The dense half of search: one int8 vector per stored chunk from the static embedding model,
//! and the nearest chunks to a query by integer dot products. Vectors are saved as one blob and
//! reused while their chunk's text is unchanged, so a restart embeds only what is new.

use metldr_embed::{Model, dot, quantize};
use std::{collections::HashMap, hash::Hasher};

const MAGIC: &[u8; 8] = b"MTLDRV01";

#[derive(Default)]
pub struct Dense {
    model: Option<Model>,
    /// One live entry per chunk id: its text's hash, scale and slot in `vectors`.
    entries: HashMap<Box<str>, Entry>,
    /// Vectors restored from a blob, waiting for their chunk to be added again.
    restored: HashMap<Box<str>, (u64, f32, Box<[i8]>)>,
    vectors: Vec<i8>,
    /// Each slot's scale beside its vector, so a search reads no map; 0 for a free slot.
    scales: Vec<f32>,
    owners: Vec<Option<Box<str>>>,
    free: Vec<usize>,
    /// Scratch kept between calls.
    ids: Vec<u32>,
    floats: Vec<f32>,
    quantized: Vec<i8>,
}

struct Entry {
    hash: u64,
    scale: f32,
    slot: usize,
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(text.as_bytes());
    hasher.finish()
}

impl Dense {
    /// Loads the packed model; false when the file is not one.
    pub fn load_model(&mut self, bytes: Vec<u8>) -> bool {
        self.model = Model::parse(bytes);
        self.model.is_some()
    }

    pub fn has_model(&self) -> bool {
        self.model.is_some()
    }

    fn dim(&self) -> usize {
        self.model.as_ref().map_or(0, Model::dim)
    }

    /// The vector for chunk `id` with `text`: restored when its text is unchanged, else
    /// embedded. Nothing without a model.
    pub fn add(&mut self, id: &str, text: &str) {
        let Some(model) = &self.model else { return };
        let hash = text_hash(text);
        if self.entries.get(id).is_some_and(|e| e.hash == hash) {
            return;
        }
        let dim = model.dim();
        let scale = match self.restored.remove(id).filter(|(h, _, v)| *h == hash && v.len() == dim) {
            Some((_, scale, vector)) => {
                self.quantized.clear();
                self.quantized.extend_from_slice(&vector);
                scale
            }
            None => {
                self.floats.resize(dim, 0.0);
                model.embed(text, &mut self.ids, &mut self.floats);
                quantize(&self.floats, &mut self.quantized)
            }
        };
        self.remove(id);
        let slot = self.free.pop().unwrap_or_else(|| {
            self.owners.push(None);
            self.scales.push(0.0);
            self.vectors.resize(self.vectors.len() + dim, 0);
            self.owners.len() - 1
        });
        self.vectors[slot * dim..][..dim].copy_from_slice(&self.quantized);
        self.owners[slot] = Some(id.into());
        self.scales[slot] = scale;
        self.entries.insert(id.into(), Entry { hash, scale, slot });
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(entry) = self.entries.remove(id) {
            self.owners[entry.slot] = None;
            self.scales[entry.slot] = 0.0;
            self.free.push(entry.slot);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.vectors.clear();
        self.scales.clear();
        self.owners.clear();
        self.free.clear();
    }

    /// The ids of the `limit` chunks nearest `query`, nearest first.
    pub fn search(&mut self, query: &str, limit: usize) -> Vec<String> {
        let Some(model) = &self.model else { return Vec::new() };
        let dim = model.dim();
        self.floats.resize(dim, 0.0);
        model.embed(query, &mut self.ids, &mut self.floats);
        let query_scale = quantize(&self.floats, &mut self.quantized);
        if query_scale == 0.0 || limit == 0 {
            return Vec::new();
        }
        // One pass over the flat vectors and scales; free slots score 0 and drop out.
        let mut scored: Vec<(f32, usize)> = self
            .vectors
            .chunks_exact(dim)
            .zip(&self.scales)
            .enumerate()
            .filter(|(_, (_, scale))| **scale != 0.0)
            .map(|(slot, (row, scale))| (dot(row, &self.quantized) as f32 * scale, slot))
            .collect();
        if scored.len() > limit {
            scored.select_nth_unstable_by(limit, |a, b| b.0.total_cmp(&a.0));
            scored.truncate(limit);
        }
        scored.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        scored.into_iter().filter_map(|(_, slot)| self.owners[slot].as_deref().map(str::to_owned)).collect()
    }

    /// Every live vector as one blob: magic, dimension and count, then per chunk its id, text
    /// hash, scale and int8 vector.
    pub fn blob(&self) -> Vec<u8> {
        let dim = self.dim();
        let mut out = Vec::with_capacity(16 + self.entries.len() * (dim + 48));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (id, entry) in &self.entries {
            out.extend_from_slice(&(id.len() as u16).to_le_bytes());
            out.extend_from_slice(id.as_bytes());
            out.extend_from_slice(&entry.hash.to_le_bytes());
            out.extend_from_slice(&entry.scale.to_le_bytes());
            out.extend(self.vectors[entry.slot * dim..][..dim].iter().map(|&v| v as u8));
        }
        out
    }

    /// Keeps a blob's vectors for their chunks to claim as they are added; a blob of another
    /// dimension or a damaged one is dropped, and those chunks are embedded again.
    pub fn restore(&mut self, blob: &[u8]) {
        self.restored.clear();
        let read = |at: &mut usize, n: usize| -> Option<&[u8]> {
            let bytes = blob.get(*at..*at + n)?;
            *at += n;
            Some(bytes)
        };
        let parse = || -> Option<HashMap<Box<str>, (u64, f32, Box<[i8]>)>> {
            let mut at = 0;
            if read(&mut at, 8)? != MAGIC {
                return None;
            }
            let dim = u32::from_le_bytes(read(&mut at, 4)?.try_into().ok()?) as usize;
            let count = u32::from_le_bytes(read(&mut at, 4)?.try_into().ok()?) as usize;
            if dim != self.dim() {
                return None;
            }
            let mut out = HashMap::with_capacity(count);
            for _ in 0..count {
                let len = u16::from_le_bytes(read(&mut at, 2)?.try_into().ok()?) as usize;
                let id = std::str::from_utf8(read(&mut at, len)?).ok()?;
                let hash = u64::from_le_bytes(read(&mut at, 8)?.try_into().ok()?);
                let scale = f32::from_le_bytes(read(&mut at, 4)?.try_into().ok()?);
                let vector: Box<[i8]> = read(&mut at, dim)?.iter().map(|&b| b as i8).collect();
                out.insert(id.into(), (hash, scale, vector));
            }
            Some(out)
        };
        self.restored = parse().unwrap_or_default();
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    fn model() -> Vec<u8> {
        let tokens = ["[UNK]", "cat", "dog", "fish"];
        metldr_embed::pack(&tokens, 0, 3, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
    }

    #[test]
    fn the_nearest_chunks_come_first_and_a_blob_restores_them_without_embedding() {
        let mut dense = Dense::default();
        assert!(dense.load_model(model()));
        dense.add("a", "cat cat dog");
        dense.add("b", "fish");
        dense.add("c", "dog");
        assert_eq!(dense.search("cat", 2), ["a", "c"]);
        let blob = dense.blob();
        let mut again = Dense::default();
        again.load_model(model());
        again.restore(&blob);
        again.add("a", "cat cat dog");
        again.add("b", "dog dog");
        assert!(again.restored.contains_key("c") && !again.restored.contains_key("a"));
        assert_eq!(again.search("dog", 1), ["b"]);
        again.remove("b");
        assert_eq!(again.search("dog", 5), ["a"]);
    }
}
