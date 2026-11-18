//! PDF reading for the extension: each page as markdown (headings, lists, tables, links) with
//! its label, and the document's outline, read by PDFium in the browser.

#[cfg(target_arch = "wasm32")]
mod read;
#[cfg(target_arch = "wasm32")]
mod wasi;

use serde::Serialize;

/// A read document as the extension takes it.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// Pages in the file, read or not.
    pub total_pages: u32,
    pub pages: Vec<Page>,
    pub outline: Vec<Heading>,
}

#[derive(Debug, Serialize)]
pub struct Page {
    /// From one.
    pub number: usize,
    /// What a reader shows for the page ("iv", "A-1"), when the file names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub markdown: String,
}

/// One outline entry: its depth from one, its title and its page from one.
#[derive(Debug, Serialize)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub page: usize,
}
