//! A block caret covers the character after it, which takes the block's ink; at a line end it
//! is a space wide, and a selection hides it.
use rune::{Color, Cx, El, Env, Runtime, TextEdit, TextStyle, text_input};

const CARET: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const INK: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
const TEXT: Color = Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };

fn view(edit: &TextEdit, _: &mut Cx<TextEdit>) -> El {
    text_input(edit, TextStyle::new(20.0, 400.0)).key("field").w(300.0).color(TEXT)
        .caret(CARET, Color { r: 0.5, g: 0.5, b: 0.5, a: 0.3 }).caret_block(INK)
        .on_edit(|edit: &mut TextEdit| edit)
}

/// The block's rects and how many glyphs take its ink, with the caret over `anchor..focus`.
fn frame(anchor: usize, focus: usize) -> (Vec<[f32; 4]>, usize) {
    let mut edit = TextEdit::new("Wi");
    (edit.anchor, edit.focus, edit.focused) = (anchor, focus, true);
    let mut rt = Runtime::new(edit, view, (320.0, 80.0), Env::new(false, 1.0));
    rt.frame(0.0);
    let is = |fill: [f32; 4], c: Color| (fill[0] - c.r).abs() < 0.01 && (fill[1] - c.g).abs() < 0.01 && (fill[2] - c.b).abs() < 0.01 && fill[3] > 0.99;
    let blocks = rt.instances.iter().filter(|i| is(i.fill, CARET)).map(|i| i.rect).collect();
    (blocks, rt.instances.iter().filter(|i| is(i.fill, INK)).count())
}

#[test]
fn block_caret_covers_the_next_character_in_its_ink() {
    let (wide, inked) = frame(0, 0);
    assert_eq!((wide.len(), inked), (1, 1), "one block over W, and W takes its ink");
    let (narrow, inked) = frame(1, 1);
    assert_eq!((narrow.len(), inked), (1, 1), "one block over i");
    assert!(wide[0][2] > narrow[0][2] + 4.0, "the block is as wide as its character: W {:?}, i {:?}", wide[0], narrow[0]);
    let (end, inked) = frame(2, 2);
    assert_eq!((end.len(), inked), (1, 0), "a line end shows a space-wide block over nothing");
    assert!((end[0][2] - 12.0).abs() < 0.5, "a space wide: {:?}", end[0]);
    assert!(end[0][3] <= 25.0 && end[0][3] > 18.0, "1.25 em tall at most: {:?}", end[0]);
    assert_eq!(frame(0, 2), (vec![], 0), "a selection hides the block");
}
