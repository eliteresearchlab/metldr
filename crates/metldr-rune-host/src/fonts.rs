//! The 13 approved subsets, decoded to SFNT by the page and handed over at start.
//!
//! Nothing is embedded in the wasm and nothing is decoded from WOFF2 here. The page checks the
//! bytes against `public/rune/fonts/manifest.json` with `scripts/check-rune-fonts.mjs`, which
//! pins each file to the digest recorded by the font-conversion evidence.

use std::cell::OnceCell;

/// Number of approved subsets: Inter (7) and JetBrains Mono (6), in the order the page loads them.
pub(super) const FONT_COUNT: usize = 13;
/// Upper bound for one file. The largest approved subset is about 190 KB.
pub(super) const MAX_FONT_BYTES: usize = 256 * 1024;
/// Sans and mono family names, as the renderer should resolve them.
pub(super) const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

thread_local! {
    static RETAINED: OnceCell<&'static [&'static [u8]]> = const { OnceCell::new() };
}

/// Refuses a font set before the renderer sees it: the count, each size, and the SFNT signature.
/// A WOFF2 file or any other format fails here rather than inside the renderer.
pub(super) fn validate(fonts: &[&[u8]]) -> Result<(), String> {
    if fonts.len() != FONT_COUNT {
        return Err(format!(
            "Expected {FONT_COUNT} font files, received {}.",
            fonts.len()
        ));
    }
    for (index, font) in fonts.iter().enumerate() {
        if font.is_empty() || font.len() > MAX_FONT_BYTES {
            return Err(format!("Font {index} has an unsupported size."));
        }
        if !is_sfnt(font) {
            return Err(format!("Font {index} is not an SFNT file."));
        }
    }
    Ok(())
}

/// TrueType (`0x00010000`), `true` and OpenType CFF (`OTTO`) signatures.
fn is_sfnt(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0, 1, 0, 0] | [b't', b'r', b'u', b'e'] | [b'O', b'T', b'T', b'O'])
    )
}

/// Keeps the renderer's font data for the document's lifetime. The renderer reads it for as long
/// as it runs, so it is leaked once: a later start in the same document reuses the first set.
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

    fn sfnt(length: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; length];
        bytes[..4].copy_from_slice(&[0, 1, 0, 0]);
        bytes
    }

    fn set(count: usize, length: usize) -> Vec<Vec<u8>> {
        (0..count).map(|_| sfnt(length)).collect()
    }

    fn views(fonts: &[Vec<u8>]) -> Vec<&[u8]> {
        fonts.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn the_approved_count_of_sfnt_files_is_accepted() {
        let fonts = set(FONT_COUNT, 1024);
        assert_eq!(validate(&views(&fonts)), Ok(()));
    }

    #[test]
    fn a_missing_or_extra_file_is_refused() {
        let short = set(FONT_COUNT - 1, 1024);
        let long = set(FONT_COUNT + 1, 1024);
        assert!(validate(&views(&short)).is_err());
        assert!(validate(&views(&long)).is_err());
    }

    #[test]
    fn empty_and_oversized_files_are_refused() {
        let mut empty = set(FONT_COUNT, 1024);
        empty[3].clear();
        assert!(validate(&views(&empty)).unwrap_err().contains("Font 3"));

        let mut huge = set(FONT_COUNT, 1024);
        huge[0] = sfnt(MAX_FONT_BYTES + 1);
        assert!(validate(&views(&huge)).unwrap_err().contains("Font 0"));
    }

    #[test]
    fn woff2_and_unknown_signatures_are_refused_before_the_renderer() {
        let mut woff2 = set(FONT_COUNT, 1024);
        woff2[5][..4].copy_from_slice(b"wOF2");
        assert!(validate(&views(&woff2)).unwrap_err().contains("Font 5"));

        let mut unknown = set(FONT_COUNT, 1024);
        unknown[12][..4].copy_from_slice(b"ABCD");
        assert!(validate(&views(&unknown)).unwrap_err().contains("Font 12"));
    }

    #[test]
    fn the_other_sfnt_signatures_are_accepted() {
        let mut fonts = set(FONT_COUNT, 1024);
        fonts[0][..4].copy_from_slice(b"true");
        fonts[1][..4].copy_from_slice(b"OTTO");
        assert_eq!(validate(&views(&fonts)), Ok(()));
    }
}
