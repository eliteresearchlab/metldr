//! Page content as markdown: the page capture's blocks (headings, paragraphs, list items, code,
//! quotes and table rows from the page's main content) rendered within the page budget. Links
//! keep their text and drop their addresses.

use serde::Deserialize;

/// Said at the end when the page goes on past the budget.
const MORE: &str = "\n\n[The page continues past this point.]";

/// What the page capture returns. Page data is untrusted: kinds, depths and sizes are checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capture {
    // Checked as present; the flow reads them from the capture itself.
    #[serde(rename = "url")]
    _url: String,
    #[serde(rename = "title")]
    _title: String,
    blocks: Vec<(String, u8, String)>,
    truncated: bool,
}

impl Capture {
    /// The blocks as markdown in at most `budget` bytes, or None for a malformed capture.
    pub(crate) fn markdown(&self, budget: usize) -> Option<String> {
        if self.blocks.len() > 4000 {
            return None;
        }
        let room = budget.checked_sub(MORE.len())?;
        let mut out = String::with_capacity(budget.min(16 * 1024));
        let mut previous: (&str, &str) = ("", "");
        let mut cut = self.truncated;
        for (kind, depth, text) in &self.blocks {
            let text = text.trim();
            if text.is_empty() || text.len() > 65_536 || (kind.as_str(), text) == previous {
                continue;
            }
            let (prefix, suffix): (std::borrow::Cow<'_, str>, &str) = match kind.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = usize::from(kind.as_bytes()[1] - b'0');
                    (format!("{} ", "#".repeat(level)).into(), "")
                }
                "p" | "row" => ("".into(), ""),
                "li" if (1..=8).contains(depth) => {
                    (format!("{}- ", "  ".repeat(usize::from(*depth) - 1)).into(), "")
                }
                "quote" => ("> ".into(), ""),
                "pre" => ("```\n".into(), "\n```"),
                _ => return None,
            };
            // List items and table rows sit on consecutive lines; everything else is a paragraph.
            let tight = kind == previous.0 && matches!(kind.as_str(), "li" | "row");
            let gap = match (out.is_empty(), tight) {
                (true, _) => "",
                (false, true) => "\n",
                (false, false) => "\n\n",
            };
            let need = gap.len() + prefix.len() + text.len() + suffix.len();
            if out.len() + need > room {
                // A first block larger than the whole budget still gives its opening.
                if out.is_empty() && suffix.is_empty() {
                    let mut end = room.saturating_sub(prefix.len());
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    out.push_str(&prefix);
                    out.push_str(&text[..end]);
                }
                cut = true;
                break;
            }
            out.push_str(gap);
            out.push_str(&prefix);
            out.push_str(text);
            out.push_str(suffix);
            previous = (kind.as_str(), text);
        }
        if out.is_empty() {
            return None;
        }
        if cut {
            out.push_str(MORE);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(blocks: serde_json::Value, truncated: bool) -> Capture {
        serde_json::from_value(serde_json::json!({"url":"https://example.com/","title":"T","blocks":blocks,"truncated":truncated})).unwrap()
    }

    #[test]
    fn blocks_render_as_markdown_within_the_budget() {
        let page = capture(serde_json::json!([
            ["h1",0,"North report"],["p",0,"The North project reports 17 units."],["p",0,"The North project reports 17 units."],
            ["li",1,"First"],["li",2,"Nested"],["row",0,"Region | Units"],["row",0,"North | 17"],
            ["quote",0,"Said once."],["pre",0,"let x = 1;\nlet y = 2;"]
        ]), false);
        assert_eq!(
            page.markdown(8192).unwrap(),
            "# North report\n\nThe North project reports 17 units.\n\n- First\n  - Nested\n\nRegion | Units\nNorth | 17\n\n> Said once.\n\n```\nlet x = 1;\nlet y = 2;\n```"
        );
        let long = capture(serde_json::json!([["p",0,"é".repeat(5000)],["p",0,"never reached"]]), false);
        let text = long.markdown(8192).unwrap();
        assert!(text.len() <= 8192 && text.ends_with(MORE) && !text.contains("never"));
        let cut = capture(serde_json::json!([["p",0,"Short."]]), true);
        assert_eq!(cut.markdown(8192).unwrap(), format!("Short.{MORE}"));
        assert!(capture(serde_json::json!([["script",0,"x"]]), false).markdown(8192).is_none());
        assert!(capture(serde_json::json!([["li",9,"too deep"]]), false).markdown(8192).is_none());
        assert!(capture(serde_json::json!([["p",0,"  "]]), false).markdown(8192).is_none());
    }
}
