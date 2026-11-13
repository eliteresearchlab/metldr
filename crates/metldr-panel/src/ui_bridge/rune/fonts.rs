//! The chat fonts, read by the page from `chat/fonts/` and handed over at start.
//!
//! Nothing is embedded in the wasm. The service worker and the DOM panel load the same module,
//! so embedded fonts cost every start about 33 MB they never use.

use std::cell::OnceCell;

/// Approved files in renderer order: 13 Inter and JetBrains Mono subsets, then 10 fallbacks.
pub(super) const NAMES: [&str; 23] = [
    "ba9851c3c22cd980-s.ttf",
    "21350d82a1f187e9-s.ttf",
    "c5fe6dc8356a8c31-s.ttf",
    "19cfc7226ec3afaa-s.ttf",
    "df0a9ae256c0569c-s.ttf",
    "8e9860b6e62d6359-s.ttf",
    "e4af272ccee01ff0-s.p.ttf",
    "886030b0b59bc5a7-s.ttf",
    "0aa834ed78bf6d07-s.ttf",
    "67957d42bae0796d-s.ttf",
    "f911b923c6adde36-s.ttf",
    "939c4f875ee75fbb-s.ttf",
    "bb3ef058b751a6ad-s.p.ttf",
    "NotoSansHebrew-Regular.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansDevanagari-Regular.ttf",
    "NotoSansBengali-Regular.ttf",
    "NotoSansThai-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "NotoSansCJK-Regular.ttc",
    "NotoColorEmoji.ttf",
];
/// Upper bound for one file. The largest approved file, the CJK collection, is about 19.5 MB.
const MAX_FONT_BYTES: usize = 24 * 1024 * 1024;

pub(super) const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

thread_local! {
    static RETAINED: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
}

/// Refuses a font set before the renderer sees it: the count, each size and the signature.
pub(super) fn validate(fonts: &[Vec<u8>]) -> Result<(), String> {
    if fonts.len() != NAMES.len() {
        return Err(format!(
            "Expected {} font files, received {}.",
            NAMES.len(),
            fonts.len()
        ));
    }
    for (font, name) in fonts.iter().zip(NAMES) {
        if font.is_empty() || font.len() > MAX_FONT_BYTES {
            return Err(format!("Font {name} has an unsupported size."));
        }
        if !matches!(
            font.get(..4),
            Some([0, 1, 0, 0] | b"true" | b"OTTO" | b"ttcf")
        ) {
            return Err(format!("Font {name} is not an SFNT file."));
        }
    }
    Ok(())
}

/// Keeps the font data for the document's lifetime, since the renderer reads it while it runs.
/// It is leaked once; a later start in the same document reuses the first set.
pub(super) fn retain(fonts: Vec<Vec<u8>>) -> &'static [&'static [u8]] {
    RETAINED.with(|retained| {
        *retained.get_or_init(|| {
            let slices: Vec<&'static [u8]> =
                fonts.into_iter().map(|font| &*Vec::leak(font)).collect();
            &*Vec::leak(slices)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(head: &[u8; 4]) -> Vec<Vec<u8>> {
        (0..NAMES.len())
            .map(|_| {
                let mut font = vec![0; 64];
                font[..4].copy_from_slice(head);
                font
            })
            .collect()
    }

    #[test]
    fn accepts_the_approved_set_and_refuses_wrong_count_or_format() {
        assert!(validate(&set(&[0, 1, 0, 0])).is_ok());
        assert!(validate(&set(b"ttcf")).is_ok());
        assert!(validate(&set(b"wOF2")).is_err());
        assert!(validate(&set(b"OTTO")[1..]).is_err());
        let mut empty = set(b"true");
        empty[3].clear();
        assert!(validate(&empty).is_err());
    }
}
