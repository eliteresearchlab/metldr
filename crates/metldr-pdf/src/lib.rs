//! PDF reading for the extension: the document as markdown (headings, lists, tables, links),
//! read by PDFium in the browser.

#[cfg(target_arch = "wasm32")]
mod read;
#[cfg(target_arch = "wasm32")]
mod wasi;

/// The `pages` that hold text, in order and a blank line apart, in one exact allocation.
pub fn joined<'a>(pages: impl Iterator<Item = &'a str> + Clone) -> String {
    let texts = pages.filter(|page| !page.trim().is_empty());
    let size = texts.clone().map(|page| page.len() + 2).sum::<usize>().saturating_sub(2);
    let mut out = String::with_capacity(size);
    for page in texts {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(page);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn pages_without_text_leave_no_gap() {
        let text = super::joined(["# One", " \n", "Two"].into_iter());
        assert_eq!((text.as_str(), text.capacity()), ("# One\n\nTwo", 10));
    }
}
