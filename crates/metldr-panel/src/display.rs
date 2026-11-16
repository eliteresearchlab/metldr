/// Strip terminal control sequences for presentation. Payloads used as selection values remain unchanged.
pub(crate) fn plain(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index += 1;
            match bytes.get(index) {
                Some(b'[') => {
                    index += 1;
                    while index < bytes.len() {
                        let byte = bytes[index];
                        index += 1;
                        if (0x40..=0x7e).contains(&byte) {
                            break;
                        }
                    }
                }
                Some(b']') => {
                    index += 1;
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                }
                Some(byte) if byte.is_ascii() => index += 1,
                _ => {}
            }
            continue;
        }
        let start = index;
        let ch = value[index..].chars().next().expect("valid UTF-8 boundary");
        index += ch.len_utf8();
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        out.push_str(&value[start..index]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn ansi_sequences_are_removed_from_display_text() {
        assert_eq!(plain("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(plain("a\u{1b}]0;title\u{7}b"), "ab");
        assert_eq!(plain("x\u{1b}[31"), "x");
        assert_eq!(plain("x\u{1b}]unfinished"), "x");
        assert_eq!(plain("\u{1b}雪 🌱"), "雪 🌱");
    }
}
