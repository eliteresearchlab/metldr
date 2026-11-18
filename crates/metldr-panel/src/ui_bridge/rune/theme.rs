//! The panel's parts of the theme: tooltips, keys and a screen's root, which reach the panel's
//! state and sounds. Colours, motion and type are the shared theme in `metldr_ui::theme`.

pub(super) use metldr_ui::theme::*;
use ::rune::{Color, El, SpringPair, Transition, Vec2, oklch, row};

/// Gives an icon-only control keyed `key` the tooltip `text`, which also names it.
pub(super) fn tip(el: El, key: &'static str, text: &'static str, state: &super::State) -> El {
    metldr_ui::tooltip::trigger(
        el,
        text,
        metldr_ui::tooltip::Type::Label,
        state.tooltip.is_some_and(|(open, _)| open == key),
        move |state: &mut super::State, open, _| {
            if open {
                state.tooltip = Some((key, text));
            } else if state.tooltip.is_some_and(|(shown, _)| shown == key) {
                state.tooltip = None;
            }
        },
    )
}

/// The open tooltip: a small chip above its trigger in the theme's tooltip colours, growing from
/// the trigger as popups do.
pub(super) fn tooltip(state: &super::State) -> Option<El> {
    let (key, text) = state.tooltip?;
    let p = state.preferences.palette();
    let typography = state.preferences.typography();
    let chip = row()
        .key("rune-tooltip")
        .px(9.0)
        .py(5.0)
        .radius(if p.neon() { 2.0 } else { 8.0 })
        .fill(p.tooltip)
        .border(1.0, if p.neon() { p.accent.alpha(0.6) } else { p.line_strong })
        .shadow(0.0, 8.0, oklch(0.0, 0.0, 0.0).alpha(0.18))
        .child(typography.legend(text).font(typography.font(11.5, 500.0)).color(p.tooltip_ink))
        .transition(Transition::scale_anchor(0.96, Vec2 { x: 0.5, y: 1.0 }).springs(SpringPair::with_exit_ratio(settle(p, 0.16, 0.22), 0.75)));
    Some(metldr_ui::tooltip::tooltip(chip, key, ::rune::Side::Top, |state: &mut super::State, _| state.tooltip = None))
}

/// A key: on a molded theme it stands on a hard 2 px lip, ringed by a hairline, and sinks onto
/// it while pressed instead of shrinking; on a flat theme
/// it keeps the outline and field fill of an outlined control.
pub(super) fn keycap(el: El, p: &Palette) -> El {
    if p.neon() {
        return neon_key(el.fill(p.surface), p.accent);
    }
    if p.retro() {
        // A retro key is stamped: a heavier hairline, softened corners, and a hard dark shadow
        // it presses into.
        return on_lip(el.radius(4.0).fill(p.surface).border(1.5, p.line_strong), p.lip, false);
    }
    let el = el.radius(if p.molded() { 6.0 } else { 8.0 }).fill(if p.molded() { p.surface } else { p.field }).border(1.0, p.line);
    if p.molded() { on_lip(el, p.lip, false) } else { el }
}

/// A neon key: sharp 2 point corners, a lit hairline in `accent` with a soft glow around it, and
/// the accent washing in while it is pressed. It sounds its down-stroke as the pointer lands.
pub(super) fn neon_key(el: El, accent: Color) -> El {
    el.radius(2.0)
        .border(1.0, accent.alpha(0.55))
        .shadow(0.0, 10.0, accent.alpha(0.22))
        .press(accent.alpha(0.16))
        .on_press_down(move |state: &mut super::State, _| state.sound(crate::interaction_audio::Cue::KeyDown(false)))
}

pub(super) fn on_lip(el: El, lip: Color, primary: bool) -> El {
    el.shadow(2.0, 0.0, lip)
        .press_travel(2.0)
        .on_press_down(move |state: &mut super::State, _| state.sound(crate::interaction_audio::Cue::KeyDown(primary)))
}

/// A screen's root on a theme's material.
pub(super) trait Root {
    /// The theme's ground under everything and its keyboard focus ring in the accent, as
    /// (width, offset) points: molded 2 at 3, neon 1.5 at 2, retro 2 at 2. A flat theme keeps
    /// the default ring and no ground.
    fn material(self, p: &Palette) -> Self;
}

impl Root for El {
    fn material(self, p: &Palette) -> Self {
        let el = self.children(super::ground::ground(p));
        let (width, offset) = match p.material {
            Material::Molded => (2.0, 3.0),
            Material::Neon => (1.5, 2.0),
            Material::Retro => (2.0, 2.0),
            Material::Flat => return el,
        };
        el.focus_ring_default(width, offset, p.accent)
    }
}


#[cfg(test)]
mod colour_source {
    //! Colours come from this file. A component that writes a colour of its own drew Rune's
    //! default white on a light theme before (the composer's text), so the check reads every
    //! Rune UI source and fails on a colour written outside it.
    const LITERALS: [&str; 7] =
        ["oklch(", "Color::WHITE", "Color::BLACK", "color::hex(", "Color::rgb", "Color::from_rgb", "Color { r"];

    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "theme.rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn every_colour_comes_from_the_theme() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("src/ui_bridge/rune");
        let mut files = Vec::new();
        sources(&dir, &mut files);
        // The shared components draw from the same theme.
        sources(&root.join("../metldr-ui/src"), &mut files);
        let mut found = Vec::new();
        for file in files {
            for (n, line) in std::fs::read_to_string(&file).unwrap().lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if LITERALS.iter().any(|literal| code.contains(literal)) {
                    found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "a colour written outside metldr-ui/src/theme.rs; add a Palette role or a theme function and read it from there:\n{}",
            found.join("\n")
        );
    }
}
