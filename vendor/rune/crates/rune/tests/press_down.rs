//! `El::on_press_down` runs as a primary pointer press lands, before the press activates, and
//! never for an unavailable control.
use rune::{Cx, El, Env, Runtime, col, el};

#[derive(Default)]
struct S { downs: u32, presses: u32 }

fn view(_: &S, _: &mut Cx<S>) -> El {
    let key = |k: &str| el().key(k).size(80.0, 40.0)
        .on_press_down(|s: &mut S, _| s.downs += 1)
        .on_press(|s: &mut S| s.presses += 1);
    col().child(key("key")).child(key("locked").unavailable(true))
}

#[test]
fn press_down_runs_on_the_down_stroke() {
    let mut rt = Runtime::new(S::default(), view, (200.0, 200.0), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.pointer_down(40.0, 20.0);
    rt.frame(0.016);
    assert_eq!((rt.state.downs, rt.state.presses), (1, 0), "down before the press activates");
    rt.pointer_up(40.0, 20.0);
    rt.frame(0.016);
    assert_eq!((rt.state.downs, rt.state.presses), (1, 1), "the release activates without a second down");
    rt.pointer_down(40.0, 60.0);
    rt.frame(0.016);
    rt.pointer_up(40.0, 60.0);
    rt.frame(0.016);
    assert_eq!(rt.state.downs, 1, "an unavailable control has no down-stroke");
}
