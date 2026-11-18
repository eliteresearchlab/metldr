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
    /// The page's own markdown, when it offers one (`link rel=alternate type=text/markdown`).
    alternate: String,
}

impl Capture {
    /// The page's own markdown address, when it is safe to fetch for `page`.
    pub(crate) fn site_markdown(&self, page: &str) -> Option<&str> {
        same_site(page, &self.alternate).then_some(self.alternate.as_str())
    }

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

/// `text` from the site, links reduced to their words, within `budget` bytes: cut at a line
/// when it is longer.
pub(crate) fn bounded(text: &str, budget: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    let text = &unlinked(text);
    if text.len() <= budget {
        return Some(text.to_owned());
    }
    let mut end = budget.checked_sub(MORE.len())?;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').unwrap_or(end);
    Some(format!("{}{MORE}", text[..end].trim_end()))
}

/// `markdown` with each `[words](address)` as its words and each image as its alt text,
/// fenced code left as written.
fn unlinked(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fenced = false;
    for line in markdown.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced || !line.contains("](") {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('[') {
            let label_end = rest[open..].find("](").map(|i| open + i);
            let close = label_end.and_then(|end| rest[end..].find(')').map(|i| end + i));
            let (Some(end), Some(close)) = (label_end, close) else { break };
            if rest[open + 1..end].contains(']') {
                // A bracket that is not a link's label: keep it and look past it.
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
                continue;
            }
            let image = rest[..open].ends_with('!');
            out.push_str(&rest[..open - usize::from(image)]);
            out.push_str(&rest[open + 1..end]);
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
    }
    out
}

/// The host of an https address without credentials, lowercased; None for anything else.
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    let named = host.contains('.') && !host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    (!authority.contains('@') && named && !host.starts_with('[')).then(|| host.to_ascii_lowercase())
}

/// The registrable part of `host`: its last two labels, or three under a two-level suffix.
fn site(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').rsplit('.').collect();
    let suffix_part = matches!(labels.get(1), Some(&("co" | "com" | "org" | "net" | "gov" | "ac" | "edu")));
    let two_level = labels.len() > 2 && suffix_part;
    let keep = if two_level { 3 } else { 2 };
    labels.iter().take(keep).rev().copied().collect::<Vec<_>>().join(".")
}

/// Whether `href` is an https address on the same site as `page`: never an IP address, a
/// loopback or single-label name, so a page cannot point the extension at a local service.
fn same_site(page: &str, href: &str) -> bool {
    let page_host = page.strip_prefix("http://").map(|rest| format!("https://{rest}"));
    let page_host = https_host(page_host.as_deref().unwrap_or(page));
    match (page_host, https_host(href)) {
        (Some(page), Some(link)) => !link.ends_with("localhost") && site(&page) == site(&link),
        _ => false,
    }
}

/// The most page text read for the gateway, in bytes: the capture's own limit.
pub(crate) const FULL_PAGE: usize = 65_536;

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(blocks: serde_json::Value, truncated: bool) -> Capture {
        serde_json::from_value(serde_json::json!({"url":"https://example.com/","title":"T","blocks":blocks,"truncated":truncated,"alternate":""})).unwrap()
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

    #[test]
    fn only_a_same_site_https_markdown_link_is_fetched() {
        let page = "https://developers.cloudflare.com/workers/";
        assert!(same_site(page, "https://developers.cloudflare.com/workers/index.md"));
        assert!(same_site("https://vercel.com/docs", "https://vercel.com/docs.md"));
        assert!(same_site("https://docs.example.co.uk/a", "https://www.example.co.uk/a.md"));
        for href in [
            "http://developers.cloudflare.com/a.md",
            "https://127.0.0.1:11434/api.md",
            "https://localhost/a.md",
            "https://user@developers.cloudflare.com/a.md",
            "https://intranet/a.md",
            "https://other.co.uk/a.md",
            "https://evil.example/a.md",
            "",
        ] {
            assert!(!same_site(page, href), "{href}");
        }
        assert!(!same_site("https://a.co.uk/", "https://b.co.uk/x.md"));
        assert_eq!(bounded("# Title\n\nBody", 8192).unwrap(), "# Title\n\nBody");
        assert_eq!(
            bounded("A [note] then [the guide](https://a.example/g) and ![chart](c.png).\n```\n[keep](x)\n```", 8192).unwrap(),
            "A [note] then the guide and chart.\n```\n[keep](x)\n```"
        );
        assert!(bounded("<!doctype html>", 8192).is_none());
        let long = format!("{}\nlast line", "line\n".repeat(3000));
        let cut = bounded(&long, 8192).unwrap();
        assert!(cut.len() <= 8192 && cut.ends_with(MORE) && !cut.contains("last line"));
    }
}
