mod chunking;
pub mod dense;
pub mod gateway;
pub mod summary_gate;
pub mod word;
pub mod dictionary;
pub mod stats;

pub use chunking::{Chunk, ContentType, chunk_for_embedding};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use metldr_retrieval::Chunk;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn chunk_for_embedding(text: &str, content_type: &str) -> Result<String, JsError> {
        let content_type = super::ContentType::try_from(content_type).map_err(JsError::new)?;
        serde_json::to_string(&super::chunk_for_embedding(text, content_type))
            .map_err(|_| JsError::new("could not encode chunks"))
    }

    /// Search over the stored chunks: keywords (`metldr_retrieval`), the static embeddings
    /// (`crate::dense`) and their fusion.
    #[wasm_bindgen]
    #[derive(Default)]
    pub struct SearchIndex {
        keywords: metldr_retrieval::Index,
        dense: crate::dense::Dense,
    }

    #[wasm_bindgen]
    impl SearchIndex {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            Self::default()
        }

        /// Takes the search model fetched by `search_model_fetch`; false when none is ready.
        pub fn use_search_model(&mut self) -> bool {
            crate::gateway::web::take_search_model().is_some_and(|bytes| self.dense.load_model(bytes))
        }

        pub fn has_model(&self) -> bool {
            self.dense.has_model()
        }

        /// Embeds an already added chunk, for chunks added before the model was ready.
        pub fn embed(&mut self, id: &str, content: &str) {
            self.dense.add(id, content);
        }

        /// Keeps a saved vector blob for the chunks added next (see `Dense::restore`).
        pub fn restore_vectors(&mut self, blob: &[u8]) {
            self.dense.restore(blob);
        }

        /// Every chunk's vector, to save.
        pub fn vectors(&self) -> Vec<u8> {
            self.dense.blob()
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
            let chunk = Chunk { id, kind, title, content, source_url, source_id, summary };
            self.keywords.add(&chunk);
            self.dense.add(id, content);
        }

        pub fn remove(&mut self, id: &str) {
            self.keywords.remove(id);
            self.dense.remove(id);
        }

        pub fn clear(&mut self) {
            self.keywords.clear();
            self.dense.clear();
        }

        #[wasm_bindgen(getter)]
        pub fn size(&self) -> u32 {
            self.keywords.len() as u32
        }

        pub fn has_source(&self, source: &str) -> bool {
            self.keywords.has_source(source)
        }

        /// The ids of the `limit` chunks nearest `query` by meaning, nearest first.
        pub fn dense(&mut self, query: &str, limit: u32) -> Vec<String> {
            self.dense.search(query, limit as usize)
        }

        /// Page chat context for `focused` (see `Index::context`); `page` may be empty.
        pub fn context(&self, focused: &str, dense: Vec<String>, limit: u32, page: &str) -> String {
            self.keywords.context(focused, &dense, limit as usize, page)
        }

        /// `{context, sources: [{index, title, url, type, score, snippet}]}` as JSON.
        pub fn sources(
            &self,
            question: &str,
            focused: &str,
            dense: Vec<String>,
            limit: u32,
        ) -> Result<String, JsError> {
            let sources = self.keywords.sources(question, focused, &dense, limit as usize);
            serde_json::to_string(&sources).map_err(|_| JsError::new("could not encode sources"))
        }
    }

    /// The topic words of a search question (see `metldr_retrieval::query::focus`).
    #[wasm_bindgen]
    pub fn focus_query(query: &str) -> String {
        metldr_retrieval::query::focus(query)
    }

    #[wasm_bindgen]
    pub fn expand_query_locally(query: &str) -> String {
        metldr_retrieval::query::expand_locally(query)
    }

    /// A model's suggested search terms, space separated (see `metldr_retrieval::query::suggested_terms`).
    #[wasm_bindgen]
    pub fn suggested_terms(answer: &str) -> String {
        metldr_retrieval::query::suggested_terms(answer).join(" ")
    }
}
