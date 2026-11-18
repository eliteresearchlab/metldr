//! Element scout (R14a): ranks a page's controls for the step an agent wants to take next,
//! so the agent can read a short list instead of every control. 529 trained weights over
//! lexical features (keyword rank, word, trigram and number overlap with the goal and the last
//! acted controls, the name's length, the control's role); no embedding model. Read-only: it
//! orders refs, it grants nothing.
use metldr_retrieval::{Index, tokenize};
use std::borrow::Cow;

mod weights;
use weights::{HIDDEN, INPUTS, WEIGHTS};

/// One control as the page outline shows it.
pub struct Control<'a> {
    pub role: &'a str,
    pub name: &'a str,
    /// Whether it takes a click; true for every ref of the outline.
    pub clickable: bool,
}

const ROLES: [&str; 16] = [
    "link",
    "button",
    "textbox",
    "combobox",
    "searchbox",
    "checkbox",
    "radio",
    "listitem",
    "img",
    "option",
    "heading",
    "cell",
    "div",
    "span",
    "label",
    "svg",
];
const BASE: usize = 15 + ROLES.len();
const _: () = assert!(BASE == INPUTS && WEIGHTS.len() == HIDDEN * INPUTS + HIDDEN * 2 + 1);

/// The distinct words of `text` as 64-bit hashes, sorted, in `out`.
fn words(text: &str, out: &mut Vec<u64>) {
    out.clear();
    // FNV-1a: two words collide about once in 2^64.
    let hash = |w: &str| {
        w.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    };
    tokenize(text, &mut |w| out.push(hash(w)));
    out.sort_unstable();
    out.dedup();
}

/// The distinct letter trigrams of `text`, lowercase and packed in a `u32`, sorted, in `out`.
fn grams(text: &str, out: &mut Vec<u32>) {
    out.clear();
    let mut window = [0u8; 3];
    let mut filled = 0;
    for b in text.bytes().map(|b| b.to_ascii_lowercase()) {
        if b == b' ' {
            filled = 0;
        } else if b.is_ascii_alphanumeric() {
            window = [window[1], window[2], b];
            filled += 1;
            if filled >= 3 {
                out.push(u32::from_be_bytes([0, window[0], window[1], window[2]]));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
}

/// How many of `items` (sorted, distinct) are in `set` (sorted, distinct).
fn shared<T: Ord>(items: &[T], set: &[T]) -> usize {
    items
        .iter()
        .filter(|item| set.binary_search(item).is_ok())
        .count()
}

fn numbers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
}

/// The keyword rank of each control for `query`: 1 for the best match, 1/2 for the next, 0 for
/// none. Each control is indexed as its role and name, its slot its place in `controls`.
fn keyword(query: &str, controls: &[Control]) -> Vec<f32> {
    let mut index = Index::default();
    let mut text = String::new();
    for control in controls {
        text.clear();
        text.push_str(control.role);
        text.push(' ');
        text.push_str(control.name);
        index.add_text(&text);
    }
    let mut score = vec![0.0f32; controls.len()];
    for (rank, &slot) in index.keyword(query, controls.len()).iter().enumerate() {
        score[slot as usize] = 1.0 / (1.0 + rank as f32);
    }
    score
}

/// One score per control for `goal`, higher is more likely the next step. `previous` names the
/// controls the agent acted on most recently, newest first.
pub fn scores(goal: &str, previous: &[&str], controls: &[Control]) -> Vec<f32> {
    let previous = previous
        .iter()
        .take(3)
        .copied()
        .collect::<Vec<_>>()
        .join("; ");
    let keyword = keyword(&format!("{goal} {previous}"), controls);
    let goal_lower = goal.to_lowercase();
    let (mut goal_words, mut goal_grams, mut previous_words) = (Vec::new(), Vec::new(), Vec::new());
    words(goal, &mut goal_words);
    grams(goal, &mut goal_grams);
    words(&previous, &mut previous_words);
    let goal_numbers: Vec<&str> = numbers(&goal_lower).collect();
    let (mut name, mut g) = (Vec::new(), Vec::new());
    let mut x = [0.0f32; INPUTS];
    controls
        .iter()
        .zip(keyword)
        .map(|(c, keyword)| {
            words(c.name, &mut name);
            let in_goal = shared(&name, &goal_words) as f32;
            let in_previous = shared(&name, &previous_words) as f32;
            grams(c.name, &mut g);
            let shared = shared(&g, &goal_grams) as f32;
            let lower = c.name.to_lowercase();
            x.fill(0.0);
            x[0] = keyword;
            x[4] = f32::from(u8::from(c.clickable));
            x[5] = f32::from(u8::from(c.name.is_empty()));
            x[6] = (1.0 + c.name.split_whitespace().count() as f32).ln();
            x[7] = if name.is_empty() {
                0.0
            } else {
                in_goal / name.len() as f32
            };
            x[8] = in_goal / goal_words.len().max(1) as f32;
            x[9] = if name.is_empty() {
                0.0
            } else {
                in_previous / name.len() as f32
            };
            x[10] = 1.0;
            x[11] = if g.is_empty() {
                0.0
            } else {
                shared / g.len() as f32
            };
            x[12] = shared / goal_grams.len().max(1) as f32;
            let number_in_goal = numbers(c.name).any(|n| goal_numbers.contains(&n));
            x[13] = f32::from(u8::from(number_in_goal));
            x[14] = f32::from(u8::from(lower.len() >= 3 && goal_lower.contains(&lower)));
            if let Some(r) = ROLES.iter().position(|r| *r == c.role) {
                x[15 + r] = 1.0;
            }
            score(&x)
        })
        .collect()
}

/// The trained network: one hidden layer of `HIDDEN` rectified units.
fn score(x: &[f32; INPUTS]) -> f32 {
    let (w1, rest) = WEIGHTS.split_at(HIDDEN * INPUTS);
    let (b1, rest) = rest.split_at(HIDDEN);
    let (w2, b2) = rest.split_at(HIDDEN);
    let hidden = w1
        .chunks_exact(INPUTS)
        .zip(b1)
        .map(|(row, b)| (row.iter().zip(x).map(|(w, x)| w * x).sum::<f32>() + b).max(0.0));
    hidden.zip(w2).map(|(h, w)| h * w).sum::<f32>() + b2[0]
}

/// The role, accessible name and ref of an outline line such as `- button "Save" [ref=e12]`.
/// A name without escapes is borrowed from the line: one SIMD scan finds its closing quote.
fn parse(line: &str) -> Option<(&str, Cow<'_, str>, bool)> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let end = rest.find([' ', ':']).unwrap_or(rest.len());
    let (role, mut rest) = rest.split_at(end);
    let mut name = Cow::Borrowed("");
    if let Some(quoted) = rest.strip_prefix(" \"") {
        let first = memchr::memchr2(b'"', b'\\', quoted.as_bytes());
        if let Some(at) = first.filter(|&at| quoted.as_bytes()[at] == b'"') {
            let after = &quoted[at + 1..];
            return Some((role, Cow::Borrowed(&quoted[..at]), after.contains("[ref=")));
        }
        let name = name.to_mut();
        let mut chars = quoted.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => name.extend(chars.next().map(|(_, escaped)| escaped)),
                '"' => {
                    rest = &quoted[i + 1..];
                    break;
                }
                c => name.push(c),
            }
        }
    }
    Some((role, name, rest.contains("[ref=")))
}

/// The controls of a page outline, best first for `goal`: each line's text without its
/// indentation, refs included.
pub fn ranked<'a>(outline: &'a str, goal: &str, previous: &[&str]) -> Vec<&'a str> {
    let lines: Vec<(&str, &str, Cow<str>)> = outline
        .lines()
        .filter_map(|text| {
            parse(text)
                .filter(|(_, _, has_ref)| *has_ref)
                .map(|(role, name, _)| (text.trim_start(), role, name))
        })
        .collect();
    let controls: Vec<Control> = lines
        .iter()
        .map(|(_, role, name)| Control {
            role,
            name,
            clickable: true,
        })
        .collect();
    let scores = scores(goal, previous, &controls);
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    order.into_iter().map(|i| lines[i].0).collect()
}

/// `lines`, the best of `total` controls, under a line saying so; `by` names who ranked them.
pub fn write_shortlist(lines: &[&str], total: usize, by: &str, out: &mut String) {
    use std::fmt::Write;
    let _ = writeln!(
        out,
        "{} of {total} controls, best first for the goal{by}; snapshot without goal for the whole page.",
        lines.len()
    );
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
}

/// The best `limit` controls of a page outline for `goal`, best first, under a line saying how
/// many there were.
pub fn shortlist(outline: &str, goal: &str, previous: &[&str], limit: usize) -> String {
    let lines = ranked(outline, goal, previous);
    let mut out = String::new();
    write_shortlist(&lines[..lines.len().min(limit)], lines.len(), "", &mut out);
    out
}
