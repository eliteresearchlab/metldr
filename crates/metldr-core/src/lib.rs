mod chunking;
mod decoder;
pub mod retrieval;
mod thinking;

pub use chunking::{Chunk, ContentType, chunk_for_embedding};
pub use decoder::Decoder;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::Decoder;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn chunk_for_embedding(text: &str, content_type: &str) -> Result<String, JsError> {
        let content_type = super::ContentType::try_from(content_type).map_err(JsError::new)?;
        serde_json::to_string(&super::chunk_for_embedding(text, content_type))
            .map_err(|_| JsError::new("could not encode chunks"))
    }

    /// The keyword half of retrieval and the fusion with the dense half (see `retrieval`).
    #[wasm_bindgen]
    pub struct KeywordIndex(super::retrieval::Index);

    #[wasm_bindgen]
    impl KeywordIndex {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            Self(super::retrieval::Index::default())
        }

        pub fn add(
            &mut self,
            id: &str,
            text: &str,
            source_url: &str,
            source_id: &str,
            summary: bool,
        ) {
            self.0.add(id, text, source_url, source_id, summary);
        }

        pub fn remove(&mut self, id: &str) {
            self.0.remove(id);
        }

        pub fn clear(&mut self) {
            self.0.clear();
        }

        #[wasm_bindgen(getter)]
        pub fn size(&self) -> u32 {
            self.0.len() as u32
        }

        pub fn has_source(&self, source: &str) -> bool {
            self.0.has_source(source)
        }

        /// `[[id, score, kind], ...]` as JSON, best first; the score is a whole number from 0 to 100.
        pub fn rank(
            &self,
            query: &str,
            dense: Vec<String>,
            keyword_limit: u32,
        ) -> Result<String, JsError> {
            let hits: Vec<_> = self
                .0
                .rank(query, &dense, keyword_limit as usize)
                .into_iter()
                .map(|hit| (hit.id, (hit.score + 0.5).floor() as u32, hit.kind.as_str()))
                .collect();
            serde_json::to_string(&hits).map_err(|_| JsError::new("could not encode hits"))
        }
    }

    #[wasm_bindgen]
    pub struct OllamaStream(Decoder);

    #[wasm_bindgen]
    impl OllamaStream {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            Self(Decoder::new())
        }

        pub fn push(&mut self, bytes: &[u8]) -> Result<String, JsError> {
            self.0.push(bytes).map_err(|error| JsError::new(&error))
        }

        pub fn finish(&mut self) -> Result<String, JsError> {
            self.0.finish().map_err(|error| JsError::new(&error))
        }

        pub fn is_done(&self) -> bool {
            self.0.is_done()
        }
    }
}
