//! The interface fonts of every Rune surface, read by its page from `chat/fonts/`: the
//! interface fonts at start, the fallbacks after the first frame.
//!
//! Nothing is embedded in the wasm. The service worker loads the same module, so embedded fonts
//! would cost it about 33 MB it never uses.

use std::cell::OnceCell;

/// Approved interface files in renderer order: the Allo variable face (optical size, weight
/// and roundness axes) and 6 JetBrains Mono subsets. The chat starts with these alone.
pub const NAMES: [&str; 7] = [
    "AlloVariable.ttf",
    "0aa834ed78bf6d07-s.ttf",
    "67957d42bae0796d-s.ttf",
    "886030b0b59bc5a7-s.ttf",
    "939c4f875ee75fbb-s.ttf",
    "bb3ef058b751a6ad-s.p.ttf",
    "f911b923c6adde36-s.ttf",
];
/// Approved fallbacks, added after the first frame so a cold open neither fetches nor parses
/// them first: Allo's oblique in each weight (named into the variable face's family, so italic
/// text finds them), Inter for the Greek, Cyrillic, Vietnamese and extended Latin Allo lacks,
/// and Noto for other scripts, symbols and emoji (about 13 MB, mostly emoji).
pub const FALLBACKS: [&str; 25] = [
    "AlloVariable-ThinItalic.otf",
    "AlloVariable-ExtraLightItalic.otf",
    "AlloVariable-LightItalic.otf",
    "AlloVariable-Italic.otf",
    "AlloVariable-MediumItalic.otf",
    "AlloVariable-SemiBoldItalic.otf",
    "AlloVariable-BoldItalic.otf",
    "AlloVariable-ExtraBoldItalic.otf",
    "AlloVariable-BlackItalic.otf",
    "ba9851c3c22cd980-s.ttf",
    "21350d82a1f187e9-s.ttf",
    "c5fe6dc8356a8c31-s.ttf",
    "19cfc7226ec3afaa-s.ttf",
    "df0a9ae256c0569c-s.ttf",
    "8e9860b6e62d6359-s.ttf",
    "e4af272ccee01ff0-s.p.ttf",
    "NotoSansHebrew-Regular.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansDevanagari-Regular.ttf",
    "NotoSansBengali-Regular.ttf",
    "NotoSansThai-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "NotoColorEmoji.ttf",
];
/// The CJK collection (about 19.5 MB), added only once the chat lays out CJK text.
pub const CJK: [&str; 1] = ["NotoSansCJK-Regular.ttc"];
/// Upper bound for one file. The largest approved file, the CJK collection, is about 19.5 MB.
const MAX_FONT_BYTES: usize = 24 * 1024 * 1024;

pub const FAMILIES: (&str, &str) = ("Allo Variable", "JetBrains Mono");

thread_local! {
    static RETAINED: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
    static RETAINED_FALLBACKS: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
    static RETAINED_CJK: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
}

/// A group of fonts the page adds after start, by its manifest name.
pub fn group(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "fallbacks" => Some(&FALLBACKS),
        "cjk" => Some(&CJK),
        _ => None,
    }
}

/// Copies the page's font buffers into the module and checks them against `names`.
#[cfg(target_arch = "wasm32")]
pub fn read(fonts: &js_sys::Array, names: &[&str]) -> Result<Vec<Vec<u8>>, String> {
    let fonts: Vec<Vec<u8>> = fonts
        .iter()
        .map(|font| js_sys::Uint8Array::new(&font).to_vec())
        .collect();
    validate(&fonts, names)?;
    Ok(fonts)
}

/// Refuses a font set before the renderer sees it: the count, each size and the signature.
pub fn validate(fonts: &[Vec<u8>], names: &[&str]) -> Result<(), String> {
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
pub fn retain(fonts: Vec<Vec<u8>>) -> &'static [&'static [u8]] {
    RETAINED.with(|retained| *retained.get_or_init(|| leak(fonts)))
}

/// Keeps a group added after start like [`retain`]. `None` when it was already kept, so a
/// document adds each group to the renderer once.
pub fn retain_group(name: &str, fonts: Vec<Vec<u8>>) -> Option<&'static [&'static [u8]]> {
    let cell = if name == "cjk" { &RETAINED_CJK } else { &RETAINED_FALLBACKS };
    cell.with(|retained| {
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
