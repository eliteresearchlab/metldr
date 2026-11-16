//! The chat fonts, read by the page from `chat/fonts/`: the interface fonts at start, the
//! fallbacks after the first frame.
//!
//! Nothing is embedded in the wasm. The service worker loads the same module, so embedded fonts
//! would cost it about 33 MB it never uses.

use std::cell::OnceCell;

/// Approved interface files in renderer order: 13 Inter and JetBrains Mono subsets. The chat
/// starts with these alone.
pub(super) const NAMES: [&str; 13] = [
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
];
/// Approved fallbacks for other scripts, symbols and emoji (about 33 MB, mostly CJK and emoji),
/// added after the first frame so a cold open neither fetches nor parses them first.
pub(super) const FALLBACKS: [&str; 10] = [
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
    static RETAINED_FALLBACKS: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
}

/// Copies the page's font buffers into the module and checks them against `names`.
pub(super) fn read(
    fonts: &js_sys::Array,
    names: &[&str],
) -> Result<Vec<Vec<u8>>, wasm_bindgen::JsValue> {
    let fonts: Vec<Vec<u8>> = fonts
        .iter()
        .map(|font| js_sys::Uint8Array::new(&font).to_vec())
        .collect();
    validate(&fonts, names).map_err(|error| crate::err(&error))?;
    Ok(fonts)
}

/// Refuses a font set before the renderer sees it: the count, each size and the signature.
pub(super) fn validate(fonts: &[Vec<u8>], names: &[&str]) -> Result<(), String> {
    if fonts.len() != names.len() {
        return Err(format!(
            "Expected {} font files, received {}.",
            names.len(),
            fonts.len()
        ));
    }
    for (font, name) in fonts.iter().zip(names) {
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

/// Keeps the interface fonts for the document's lifetime, since the renderer reads them while
/// it runs. They are leaked once; a later start in the same document reuses the first set.
pub(super) fn retain(fonts: Vec<Vec<u8>>) -> &'static [&'static [u8]] {
    RETAINED.with(|retained| *retained.get_or_init(|| leak(fonts)))
}

/// Keeps the fallbacks like [`retain`]. `None` when they were already kept, so a document adds
/// them to the renderer once.
pub(super) fn retain_fallbacks(fonts: Vec<Vec<u8>>) -> Option<&'static [&'static [u8]]> {
    RETAINED_FALLBACKS.with(|retained| {
        let mut fresh = false;
        let kept = *retained.get_or_init(|| {
            fresh = true;
            leak(fonts)
        });
        fresh.then_some(kept)
    })
}

fn leak(fonts: Vec<Vec<u8>>) -> &'static [&'static [u8]] {
    let slices: Vec<&'static [u8]> = fonts.into_iter().map(|font| &*Vec::leak(font)).collect();
    Vec::leak(slices)
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
        assert!(validate(&set(&[0, 1, 0, 0]), &NAMES).is_ok());
        assert!(validate(&set(b"ttcf"), &NAMES).is_ok());
        assert!(validate(&set(b"wOF2"), &NAMES).is_err());
        assert!(validate(&set(b"OTTO")[1..], &NAMES).is_err());
        assert!(validate(&set(b"OTTO"), &FALLBACKS).is_err());
        let mut empty = set(b"true");
        empty[3].clear();
        assert!(validate(&empty, &NAMES).is_err());
    }
}
