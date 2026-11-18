# LiteParse

Source: https://github.com/run-llama/liteparse, `crates/liteparse` at commit a684a4a (2.20.0), Apache-2.0 (LICENSE).

Vendored: `crates/liteparse` (sources). PDFium and its Rust bindings come from crates.io
(`liteparse-pdfium` and `liteparse-pdfium-sys` 1.14.0); the PDFium library itself is the
`chromium/8058` wasm build, fetched by `scripts/build-rust.mjs` and used only when its SHA-256
matches the pinned value.

Changes from upstream:
- `markdown_layout/repetition.rs`: single-page header and footer detection matches each line
  against the chrome patterns once, not once per pair of lines (the pass was quadratic).
- `projection.rs`: font-name hints are found in one pass over the name instead of a substring
  search per hint for every text item.
- `markdown_layout/paragraphs.rs`: whitespace collapses by copying whole words.
- `conversion.rs`: the browser build reads PDFs only and refuses other inputs, so the image and
  office converters stay out of the module.
- `Cargo.toml`: a standalone crate; PDFium from crates.io pinned to 1.14.0.
- Left out: the `lit` command line tool, the Tesseract and OAR OCR backends, the tests,
  examples and benchmarks.

Each change keeps the markdown output byte-identical on the upstream test PDFs and the four
reference papers used to measure it.
