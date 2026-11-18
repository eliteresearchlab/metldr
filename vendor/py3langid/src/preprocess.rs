use std::borrow::Cow;

use unicode_normalization::UnicodeNormalization;

/// The titlecase letters (general category Lt); the standard library has no category lookup, and
/// this short list is all of them in Unicode 16.
fn is_titlecase(letter: char) -> bool {
    matches!(letter as u32, 0x01C5 | 0x01C8 | 0x01CB | 0x01F2 | 0x1F88..=0x1F8F | 0x1F98..=0x1F9F
        | 0x1FA8..=0x1FAF | 0x1FBC | 0x1FCC | 0x1FFC)
}

/// Text as the model reads it: NFC, and lowercased when every cased letter is a capital. Upstream
/// used ICU for the case tests and mapping; the standard library's Lowercase and Uppercase
/// properties and its lowercase mapping are the same Unicode data, without a second copy.
pub(crate) fn encode(text: &[u8]) -> Cow<'_, [u8]> {
    let valid = std::str::from_utf8(text).ok().or_else(|| {
        (1..=text.len().min(3))
            .find_map(|trim| std::str::from_utf8(&text[..text.len() - trim]).ok())
    });
    let Some(valid) = valid else {
        return Cow::Borrowed(text);
    };
    let mut all_upper = false;
    for letter in valid.chars() {
        if letter.is_lowercase() || is_titlecase(letter) {
            all_upper = false;
            break;
        }
        if letter.is_uppercase() {
            all_upper = true;
        }
    }
    let lowered = if all_upper {
        Cow::Owned(valid.to_lowercase())
    } else {
        Cow::Borrowed(valid)
    };
    Cow::Owned(
        lowered
            .chars()
            .stream_safe()
            .nfc()
            .collect::<String>()
            .into_bytes(),
    )
}
