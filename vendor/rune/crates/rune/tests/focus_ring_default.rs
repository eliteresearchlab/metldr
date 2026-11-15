//! `El::focus_ring_default` replaces the default focus ring for every control inside it that
//! sets none of its own; a control's own ring still wins.
use rune::{Color, Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el};

const THEME: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const OWN: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

fn view(_: &u8, _: &mut Cx<u8>) -> El {
    col().padding(20.0).gap(20.0).focus_ring_default(2.0, 3.0, THEME)
        .child(el().key("plain").size(60.0, 20.0).focusable(true).on_press(|_: &mut u8| {}))
        .child(el().key("own").size(60.0, 20.0).focusable(true).on_press(|_: &mut u8| {}).focus_ring(1.0, 0.0, OWN))
}

/// The ring rects drawn in `c` (an outline instance carries its colour on the border).
fn rings(rt: &Runtime<u8>, c: Color) -> Vec<[f32; 4]> {
    rt.instances.iter().filter(|i| (i.border[0] - c.r).abs() < 0.01 && (i.border[2] - c.b).abs() < 0.01 && i.border[3] > 0.99).map(|i| i.rect).collect()
}

#[test]
fn a_theme_ring_replaces_the_default_but_not_a_controls_own() {
    let mut rt = Runtime::new(0, view, (200.0, 200.0), Env::new(false, 1.0));
    rt.frame(0.0);
    let tab = |rt: &mut Runtime<u8>| { rt.key(KeyEvent { key: Key::Tab, mods: Mods::default() }); rt.frame(0.016); rt.frame(0.5); };
    tab(&mut rt);
    assert_eq!(rings(&rt, THEME), vec![[15.0, 15.0, 70.0, 30.0]], "2 points wide, 3 points out, around the plain control");
    tab(&mut rt);
    assert!(rings(&rt, THEME).is_empty(), "the control with its own ring draws that instead");
    assert_eq!(rings(&rt, OWN).len(), 1);
}
