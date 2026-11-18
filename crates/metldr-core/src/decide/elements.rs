//! Element scout (R14a): ranks a page's controls for the step an agent wants to take next,
//! so the agent can read a short list instead of every control. 785 trained weights over
//! lexical features (keyword rank, word, trigram and number overlap with the goal, the control's
//! role and its parent's role); no embedding model. Read-only: it orders refs, it grants nothing.
use metldr_retrieval::{Chunk, Index, tokenize};
use std::collections::HashSet;

mod weights;
use weights::{HIDDEN, INPUTS, WEIGHTS};

/// One control as the page outline shows it.
pub struct Control<'a> {
    pub role: &'a str,
    pub name: &'a str,
    /// The role of the outline line that contains it; empty at the top level.
    pub parent: &'a str,
    /// Whether it takes a click; true for every ref of the outline.
    pub clickable: bool,
}

const ROLES: [&str; 16] = [
    "link", "button", "textbox", "combobox", "searchbox", "checkbox", "radio", "listitem", "img",
    "option", "heading", "cell", "div", "span", "label", "svg",
];
const BASE: usize = 15 + ROLES.len() * 2;
const _: () = assert!(BASE == INPUTS && WEIGHTS.len() == HIDDEN * INPUTS + HIDDEN * 2 + 1);

fn words(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    tokenize(text, &mut |w| {
        out.insert(w.to_owned());
    });
    out
}

fn grams(text: &str) -> HashSet<[u8; 3]> {
    let lower: Vec<u8> = text
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .filter(|b| b.is_ascii_alphanumeric() || *b == b' ')
        .collect();
    lower.windows(3).filter(|w| !w.contains(&b' ')).map(|w| [w[0], w[1], w[2]]).collect()
}

fn numbers(text: &str) -> HashSet<&str> {
    text.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).collect()
}

/// The keyword rank of each control for `query`: 1 for the best match, 1/2 for the next, 0 for none.
fn keyword(query: &str, texts: &[String]) -> Vec<f32> {
    let mut index = Index::default();
    let ids: Vec<String> = (0..texts.len()).map(|i| i.to_string()).collect();
    for (id, content) in ids.iter().zip(texts) {
        index.add(&Chunk { id, kind: "page", title: "", content, source_url: id, source_id: id, summary: false });
    }
    let mut score = vec![0.0f32; texts.len()];
    for (rank, &slot) in index.keyword(query, texts.len()).iter().enumerate() {
        if let Ok(i) = index.id(slot).parse::<usize>() {
            score[i] = 1.0 / (1.0 + rank as f32);
        }
    }
    score
}

/// One score per control for `goal`, higher is more likely the next step. `previous` names the
/// controls the agent acted on most recently, newest first.
pub fn scores(goal: &str, previous: &[&str], controls: &[Control]) -> Vec<f32> {
    let previous = previous.iter().take(3).copied().collect::<Vec<_>>().join("; ");
    let texts: Vec<String> = controls.iter().map(|c| format!("{} {}", c.role, c.name)).collect();
    let keyword = keyword(&format!("{goal} {previous}"), &texts);
    let goal_lower = goal.to_lowercase();
    let (goal_words, goal_grams, previous_words) = (words(goal), grams(goal), words(&previous));
    let goal_numbers = numbers(&goal_lower);
    let mut x = [0.0f32; INPUTS];
    controls
        .iter()
        .zip(keyword)
        .map(|(c, keyword)| {
            let name = words(c.name);
            let in_goal = name.iter().filter(|w| goal_words.contains(*w)).count() as f32;
            let in_previous = name.iter().filter(|w| previous_words.contains(*w)).count() as f32;
            let g = grams(c.name);
            let shared = g.iter().filter(|t| goal_grams.contains(*t)).count() as f32;
            let lower = c.name.to_lowercase();
            x.fill(0.0);
            x[0] = keyword;
            x[4] = f32::from(u8::from(c.clickable));
            x[5] = f32::from(u8::from(c.name.is_empty()));
            x[6] = (1.0 + c.name.split_whitespace().count() as f32).ln();
            x[7] = if name.is_empty() { 0.0 } else { in_goal / name.len() as f32 };
            x[8] = in_goal / goal_words.len().max(1) as f32;
            x[9] = if name.is_empty() { 0.0 } else { in_previous / name.len() as f32 };
            x[10] = 1.0;
            x[11] = if g.is_empty() { 0.0 } else { shared / g.len() as f32 };
            x[12] = shared / goal_grams.len().max(1) as f32;
            x[13] = f32::from(u8::from(numbers(c.name).iter().any(|n| goal_numbers.contains(n))));
            x[14] = f32::from(u8::from(lower.len() >= 3 && goal_lower.contains(&lower)));
            if let Some(r) = ROLES.iter().position(|r| *r == c.role) {
                x[15 + r] = 1.0;
            }
            if let Some(r) = ROLES.iter().position(|r| *r == c.parent) {
                x[15 + ROLES.len() + r] = 1.0;
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
    let hidden = w1.chunks_exact(INPUTS).zip(b1).map(|(row, b)| {
        (row.iter().zip(x).map(|(w, x)| w * x).sum::<f32>() + b).max(0.0)
    });
    hidden.zip(w2).map(|(h, w)| h * w).sum::<f32>() + b2[0]
}

/// One parsed outline line that carries a ref.
struct Line<'a> {
    text: &'a str,
    role: &'a str,
    name: String,
    parent: &'a str,
}

/// The role, accessible name and ref of an outline line such as `- button "Save" [ref=e12]`.
fn parse(line: &str) -> Option<(&str, String, bool)> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let end = rest.find([' ', ':']).unwrap_or(rest.len());
    let (role, mut rest) = rest.split_at(end);
    let mut name = String::new();
    if let Some(quoted) = rest.strip_prefix(" \"") {
        let mut chars = quoted.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    if let Some((_, escaped)) = chars.next() {
                        name.push(escaped);
                    }
                }
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

/// The best `limit` controls of a page outline for `goal`, best first, under a line saying how
/// many there were. Lines keep their text, refs included, without their indentation.
pub fn shortlist(outline: &str, goal: &str, previous: &[&str], limit: usize) -> String {
    let mut parents: Vec<(usize, &str)> = Vec::new();
    let mut lines = Vec::new();
    for text in outline.lines() {
        let indent = text.len() - text.trim_start().len();
        let Some((role, name, has_ref)) = parse(text) else { continue };
        while parents.last().is_some_and(|(depth, _)| *depth >= indent) {
            parents.pop();
        }
        let parent = parents.last().map_or("", |(_, role)| *role);
        if has_ref {
            lines.push(Line { text: text.trim_start(), role, name, parent });
        }
        parents.push((indent, role));
    }
    let controls: Vec<Control> = lines
        .iter()
        .map(|l| Control { role: l.role, name: &l.name, parent: l.parent, clickable: true })
        .collect();
    let scores = scores(goal, previous, &controls);
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let shown = order.len().min(limit);
    let mut out = format!(
        "{shown} of {} controls, best first for the goal; snapshot without goal for the whole page.\n",
        lines.len()
    );
    for &i in &order[..shown] {
        out.push_str(lines[i].text);
        out.push('\n');
    }
    out
}
