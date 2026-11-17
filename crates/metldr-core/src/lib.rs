mod chunking;
pub mod gateway;
pub mod query;
pub mod retrieval;

pub use chunking::{Chunk, ContentType, chunk_for_embedding};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::retrieval::Chunk;
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

        #[allow(clippy::too_many_arguments)]
        pub fn add(
            &mut self,
            id: &str,
            kind: &str,
            title: &str,
            content: &str,
            source_url: &str,
            source_id: &str,
            summary: bool,
        ) {
            let chunk = Chunk {
                id,
                kind,
                title,
                content,
                source_url,
                source_id,
                summary,
            };
            self.0.add(&chunk);
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

        /// Page chat context for `focused` (see `Index::context`); `page` may be empty.
        pub fn context(&self, focused: &str, dense: Vec<String>, limit: u32, page: &str) -> String {
            self.0.context(focused, &dense, limit as usize, page)
        }

        /// `{context, sources: [{index, title, url, type, score, snippet}]}` as JSON.
        pub fn sources(
            &self,
            question: &str,
            focused: &str,
            dense: Vec<String>,
            limit: u32,
        ) -> Result<String, JsError> {
            let sources = self.0.sources(question, focused, &dense, limit as usize);
            serde_json::to_string(&sources).map_err(|_| JsError::new("could not encode sources"))
        }
    }

    /// The topic words of a search question (see `query::focus`).
    #[wasm_bindgen]
    pub fn focus_query(query: &str) -> String {
        super::query::focus(query)
    }

    #[wasm_bindgen]
    pub fn expand_query_locally(query: &str) -> String {
        super::query::expand_locally(query)
    }

    /// A model's suggested search terms, space separated (see `query::suggested_terms`).
    #[wasm_bindgen]
    pub fn suggested_terms(answer: &str) -> String {
        super::query::suggested_terms(answer).join(" ")
    }
}
