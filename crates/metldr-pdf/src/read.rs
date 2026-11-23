use liteparse::{LiteParse, LiteParseConfig, OutputFormat, types::PdfInput};
use wasm_bindgen::prelude::*;

/// Reads the PDF in `bytes`, at most `max_pages` pages, to its markdown.
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
    Ok(super::joined(read.pages.iter().map(|page| page.markdown.as_str())))
}
