use super::{Document, Heading, Page};
use liteparse::{LiteParse, LiteParseConfig, OutputFormat, types::PdfInput};
use wasm_bindgen::prelude::*;

/// Reads the PDF in `bytes`, at most `max_pages` pages, to `{totalPages, pages, outline}`.
#[wasm_bindgen]
pub async fn pdf_read(bytes: Vec<u8>, max_pages: u32) -> Result<String, JsError> {
    let config = LiteParseConfig {
        output_format: OutputFormat::Markdown,
        max_pages: max_pages.max(1) as usize,
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    };
    let read = LiteParse::new(config)
        .parse_input(PdfInput::Bytes(bytes))
        .await
        .map_err(|error| JsError::new(&format!("This PDF could not be read: {error}")))?;
    let outline = read.outline.into_iter().filter(|h| h.page_index >= 0);
    let document = Document {
        total_pages: read.total_pages,
        pages: read
            .pages
            .into_iter()
            .map(|p| Page { number: p.page_number, label: p.page_label, markdown: p.markdown })
            .collect(),
        outline: outline
            .map(|h| Heading { level: h.level, title: h.title, page: h.page_index as usize + 1 })
            .collect(),
    };
    serde_json::to_string(&document).map_err(|_| JsError::new("This PDF could not be read."))
}
