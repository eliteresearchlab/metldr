//! The one source of colour for every Rune surface (docs: design/system.md).
//!
//! Components read roles from here and never write colour literals of their own.

use ::rune::{
    Anim, AnimExt, Color, Curve, El, Easing, Spring, SpringPair, Transition, Vec2, oklch,
};

/// Semantic colour roles for one theme.
#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) page: Color,
    pub(super) canvas: Color,
    pub(super) surface: Color,
    pub(super) field: Color,
    pub(super) hover: Color,
    pub(super) hover_2: Color,
    pub(super) ink: Color,
    pub(super) ink_2: Color,
    pub(super) ink_3: Color,
    pub(super) line: Color,
    pub(super) line_strong: Color,
    pub(super) accent: Color,
    pub(super) green: Color,
    pub(super) orange: Color,
    pub(super) red: Color,
    /// The overlay a pressed control shows.
    pub(super) press: Color,
    /// The overlay a pressed control with a solid accent fill shows.
    pub(super) press_solid: Color,
    /// Text caret and selection in fields.
    pub(super) selection: Color,
    pub(super) light: bool,
    /// Molded material: modules shade from a lighter top with a lit top edge, and keys stand on
    /// a hard lip they sink onto when pressed. Flat themes draw neither.
    pub(super) molded: bool,
    /// Neon material: a dark HUD over scanlines, keys outlined by a lit hairline that glows,
    /// sharp corners, uppercase monospaced legends and synth voices.
    pub(super) neon: bool,
    /// A key's lip below it, and a module's lit top edge.
    pub(super) lip: Color,
    pub(super) highlight: Color,
    /// A legend on a key in the accent: near-white on a key darker than 0.8 lightness, ink on a
    /// lighter one (yellow), so it always reads.
    pub(super) on_accent: Color,
    /// A molded theme's gray LCD: its glass from top to bottom and the dark print on it.
    pub(super) glass: [Color; 3],
    pub(super) print: Color,
}

pub(super) type Lch = (f32, f32, f32);

/// `lch` with its chroma lowered until it fits sRGB, keeping its lightness and hue: a key colour
/// that reaches into P3 keeps its hue on an sRGB canvas instead of clipping toward another
/// (an orange-red clipped channel by channel turns red).
pub(super) fn in_srgb((l, c, h): Lch) -> Lch {
    let fits = |c: f32| {
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        [
            4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
            -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
            -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
        ]
        .iter()
        .all(|v| (-1e-4..=1.0001).contains(v))
    };
    if fits(c) {
        return (l, c, h);
    }
    let (mut lo, mut hi) = (0.0, c);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) { lo = mid } else { hi = mid }
    }
    (l, lo, h)
}

/// One theme's roles as oklch triples, in the order of [`Palette`]'s first fifteen fields, then
/// the lip and highlight of molded material.
struct Ramp {
    page: Lch,
    canvas: Lch,
    surface: Lch,
    field: Lch,
    hover: Lch,
    hover_2: Lch,
    ink: Lch,
    ink_2: Lch,
    ink_3: Lch,
    line: Lch,
    line_strong: Lch,
    accent: Lch,
    green: Lch,
    orange: Lch,
    red: Lch,
    /// A key's lip and a module's lit top edge.
    lip: Lch,
    highlight: Lch,
}

const LIGHT: Ramp = Ramp {
    page: (0.985, 0.001, 286.376),
    canvas: (0.961, 0.002, 247.84),
    surface: (1.0, 0.0, 0.0),
    field: (0.961, 0.001, 286.375),
    hover: (0.97, 0.002, 247.839),
    hover_2: (0.933, 0.003, 247.86),
    ink: (0.247, 0.006, 258.361),
    ink_2: (0.506, 0.01, 264.477),
    ink_3: (0.695, 0.009, 264.505),
    line: (0.946, 0.003, 264.542),
    line_strong: (0.912, 0.005, 258.326),
    accent: (0.626, 0.205, 254.947),
    green: (0.603, 0.155, 150.883),
    orange: (0.689, 0.179, 49.902),
    red: (0.621, 0.192, 23.042),
    lip: (0.912, 0.005, 258.326),
    highlight: (1.0, 0.0, 0.0),
};

// 0.318 is a lightness, not an approximation of 1/π.
#[allow(clippy::approx_constant)]
const DARK: Ramp = Ramp {
    page: (0.209, 0.004, 264.477),
    canvas: (0.231, 0.004, 264.487),
    surface: (0.26, 0.006, 271.191),
    field: (0.293, 0.006, 271.223),
    hover: (0.289, 0.006, 271.22),
    hover_2: (0.318, 0.007, 274.747),
    ink: (0.964, 0.002, 247.839),
    ink_2: (0.731, 0.008, 260.731),
    ink_3: (0.541, 0.01, 264.484),
    line: (0.308, 0.006, 258.354),
    line_strong: (0.356, 0.007, 264.474),
    accent: (0.68, 0.173, 253.301),
    green: (0.705, 0.154, 153.814),
    orange: (0.746, 0.156, 55.642),
    red: (0.666, 0.18, 21.433),
    lip: (0.356, 0.007, 264.474),
    highlight: (0.26, 0.006, 271.191),
};

const GRUVBOX: Ramp = Ramp {
    page: (0.24, 0.025, 75.0),
    canvas: (0.26, 0.025, 75.0),
    surface: (0.3, 0.03, 75.0),
    field: (0.33, 0.03, 75.0),
    hover: (0.33, 0.03, 75.0),
    hover_2: (0.36, 0.03, 75.0),
    ink: (0.91, 0.04, 85.0),
    ink_2: (0.74, 0.045, 80.0),
    ink_3: (0.65, 0.045, 80.0),
    line: (0.38, 0.03, 75.0),
    line_strong: (0.44, 0.035, 75.0),
    accent: (0.8, 0.15, 85.0),
    green: (0.72, 0.13, 125.0),
    orange: (0.72, 0.16, 55.0),
    red: (0.64, 0.19, 28.0),
    lip: (0.44, 0.035, 75.0),
    highlight: (0.3, 0.03, 75.0),
};

/// Light molded panels: paper ground, metal modules, keys standing on their lips, and a black
/// key colour unless the user picks one.
const HARDWARE: Ramp = Ramp {
    page: (0.911, 0.007, 106.5),
    canvas: (0.951, 0.005, 106.5),
    surface: (0.978, 0.004, 106.5),
    field: (0.955, 0.004, 91.4),
    hover: (0.939, 0.007, 106.5),
    hover_2: (0.915, 0.005, 95.1),
    ink: (0.221, 0.004, 106.7),
    ink_2: (0.448, 0.01, 106.8),
    ink_3: (0.632, 0.009, 106.6),
    line: (0.862, 0.008, 106.6),
    line_strong: (0.831, 0.008, 106.6),
    accent: (0.24, 0.0, 0.0),
    green: (0.623, 0.149, 153.1),
    orange: (0.64, 0.31, 48.0),
    red: (0.6, 0.22, 25.0),
    lip: (0.797, 0.008, 106.6),
    highlight: (1.0, 0.0, 0.0),
};

const CYBERPUNK: Ramp = Ramp {
    page: (0.17, 0.045, 290.0),
    canvas: (0.19, 0.045, 290.0),
    surface: (0.23, 0.06, 290.0),
    field: (0.26, 0.06, 290.0),
    hover: (0.26, 0.06, 290.0),
    hover_2: (0.29, 0.06, 290.0),
    ink: (0.94, 0.04, 195.0),
    ink_2: (0.78, 0.07, 195.0),
    ink_3: (0.62, 0.07, 290.0),
    line: (0.31, 0.06, 290.0),
    line_strong: (0.37, 0.07, 290.0),
    accent: (0.82, 0.14, 195.0),
    green: (0.8, 0.18, 150.0),
    orange: (0.76, 0.17, 60.0),
    red: (0.66, 0.24, 15.0),
    lip: (0.37, 0.07, 290.0),
    highlight: (0.23, 0.06, 290.0),
};

impl Palette {
    pub(super) fn for_theme(theme: &str) -> Self {
        Self::with_key(theme, None)
    }

    /// The palette for `theme`; a molded theme's keys take `key` (OKLCH) in place of its own.
    pub(super) fn with_key(theme: &str, key: Option<Lch>) -> Self {
        let ramp = match theme {
            "light" => &LIGHT,
            "gruvbox" => &GRUVBOX,
            "cyberpunk" => &CYBERPUNK,
            "hardware" => &HARDWARE,
            _ => &DARK,
        };
        let c = |(l, c, h): Lch| oklch(l, c, h);
        let molded = std::ptr::eq(ramp, &HARDWARE);
        let light = std::ptr::eq(ramp, &LIGHT) || molded;
        let accent = key.filter(|_| molded).unwrap_or(ramp.accent);
        Self {
            page: c(ramp.page),
            canvas: c(ramp.canvas),
            surface: c(ramp.surface),
            field: c(ramp.field),
            hover: c(ramp.hover),
            hover_2: c(ramp.hover_2),
            ink: c(ramp.ink),
            ink_2: c(ramp.ink_2),
            ink_3: c(ramp.ink_3),
            line: c(ramp.line),
            line_strong: c(ramp.line_strong),
            accent: c(accent),
            green: c(ramp.green),
            orange: c(ramp.orange),
            red: c(ramp.red),
            // A neutral overlay that reads on both grounds; darker ink on light themes.
            press: c(ramp.ink).alpha(if light { 0.08 } else { 0.12 }),
            press_solid: c(ramp.surface).alpha(0.16),
            selection: c(accent).alpha(0.3),
            light,
            molded,
            neon: std::ptr::eq(ramp, &CYBERPUNK),
            lip: c(ramp.lip),
            highlight: c(ramp.highlight),
            on_accent: if accent.0 < 0.8 { oklch(0.98, 0.0, 0.0) } else { oklch(0.18, 0.0, 0.0) },
            glass: ["#b6b7b1", "#c8c9c3", "#c2c3bd"].map(|hex| ::rune::color::hex(hex).unwrap_or(Color::WHITE)),
            print: ::rune::color::hex("#141413").unwrap_or(Color::BLACK),
        }
    }

    /// Distinct series colours for charts, spaced by hue at the accent's lightness and chroma.
    pub(super) fn series(&self, index: usize) -> Color {
        const HUES: [f32; 8] = [254.9, 150.9, 49.9, 300.0, 23.0, 195.0, 85.0, 330.0];
        let l = if self.light { 0.62 } else { 0.72 };
        oklch(l, 0.15, HUES[index % HUES.len()])
    }
}

/// A pressed button scales to this over 150 ms; disabled controls,
/// menu rows and list rows never scale, and Reduce Motion turns it off.
pub(super) const PRESS_SCALE: f32 = 0.96;

/// Links, spatial moves and the home reveal.
pub(super) const EASE_LINK: Easing = Easing::cubic(0.16, 1.0, 0.3, 1.0);

/// The home reveal: each line rises 23 px out of a 17 px blur over 800 ms, staged by these delays in seconds. Only the
/// welcome copy reveals; controls are usable from the first frame.
pub(super) const REVEAL_GREETING: f32 = 0.17;
pub(super) const REVEAL_QUESTION: f32 = 0.33;

/// Shows `el` as part of the home reveal: hidden on its first frame, then eased in after `delay`.
/// Reduce Motion passes `shown` from the first frame, so nothing moves.
pub(super) fn reveal(el: El, shown: bool, delay: f32) -> El {
    let anim = || Anim::new(Curve::tween(0.8, EASE_LINK)).delay(delay);
    el.opacity(if shown { 1.0 } else { 0.0 })
        .offset(0.0, if shown { 0.0 } else { 23.0 })
        .blur(if shown { 0.0 } else { 17.0 })
        .opacity_motion(anim())
        .offset_motion(anim())
        .blur_motion(anim())
}

/// A molded control snapping to its next stop, slow off the mark and decisive into place.
pub(super) const EASE_DETENT: Easing = Easing::cubic(0.45, 0.0, 0.15, 1.0);

/// Moves `el`'s offset to its next stop as a molded detent does: 120 ms on [`EASE_DETENT`]. Under
/// Reduce Motion the offset lands at once.
pub(super) fn detent(el: El) -> El {
    el.offset_motion(Anim::new(Curve::tween(0.12, EASE_DETENT)))
}

/// How long a surface takes to settle into place: a molded theme's parts are heavier, so its
/// popups take 260 ms and its sheets 300 ms on a critically damped spring, fast off the mark
/// and landing softly.
fn settle(p: &Palette, flat: f32, molded: f32) -> Spring {
    // A neon HUD snaps: its surfaces land in three quarters of a flat theme's time.
    Spring::from_motion(if p.molded { molded } else if p.neon { flat * 0.75 } else { flat }, 0.0)
}

/// A menu that opens above its trigger grows from the trigger's
/// corner, 0.95 to 1 with opacity, on a critically damped 160 ms spring (260 ms molded); the
/// exit is shorter. Under Reduce Motion Rune turns it into a short fade without the scale.
pub(super) fn pop_in(p: &Palette) -> Transition {
    let enter = settle(p, 0.16, 0.26);
    Transition::scale_anchor(0.95, Vec2 { x: 0.0, y: 1.0 })
        .springs(SpringPair::with_exit_ratio(enter, 0.75))
}

/// A new message fades in rising 8 px, on a critically damped 300 ms spring; the exit is shorter.
/// Under Reduce Motion Rune turns it into a short fade without the rise.
pub(super) fn rise() -> Transition {
    rise_over(0.3)
}

/// Fades in rising 8 px on a critically damped spring of `seconds`; the exit is shorter.
fn rise_over(seconds: f32) -> Transition {
    Transition::offset(0.0, 8.0)
        .combined(Transition::opacity())
        .springs(SpringPair::with_exit_ratio(Spring::from_motion(seconds, 0.0), 0.75))
}

/// A sheet such as Saved chats opens from its centre, 0.96 to 1 with opacity, on a critically damped
/// 250 ms spring (300 ms molded); the exit is shorter. Under Reduce Motion Rune turns it into a
/// short fade without the scale.
pub(super) fn sheet(p: &Palette) -> Transition {
    Transition::scale_anchor(0.96, Vec2 { x: 0.5, y: 0.5 })
        .springs(SpringPair::with_exit_ratio(settle(p, 0.25, 0.3), 0.75))
}

/// A reply card: surface, 10 px radius, hairline ring and a soft shadow, 12 px padding. It fades
/// in rising 8 px on a critically damped 380 ms spring; Reduce Motion keeps only the fade.
pub(super) fn card(palette: &Palette) -> El {
    ::rune::col()
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(10.0)
        .fill(palette.surface)
        .border(1.0, palette.line)
        .shadow(1.0, 2.0, oklch(0.0, 0.0, 0.0).alpha(0.05))
        .transition(rise_over(0.38))
}

/// A key: on a molded theme it stands on a hard 2 px lip, ringed by a hairline, and sinks onto
/// it while pressed instead of shrinking; on a flat theme
/// it keeps the outline and field fill of an outlined control.
pub(super) fn keycap(el: El, p: &Palette) -> El {
    if p.neon {
        return neon(el, p);
    }
    let el = el.radius(if p.molded { 6.0 } else { 8.0 }).fill(if p.molded { p.surface } else { p.field }).border(1.0, p.line);
    if p.molded { on_lip(el, p.lip, false) } else { el }
}

/// A neon key: sharp 2 point corners, a lit hairline in the accent with a soft glow around it,
/// and the accent washing in while it is pressed. It sounds its down-stroke as the pointer lands.
pub(super) fn neon(el: El, p: &Palette) -> El {
    neon_key(el.fill(p.surface), p.accent)
}

/// A neon key's outline, glow, pressed wash and down-stroke, in `accent`.
pub(super) fn neon_key(el: El, accent: Color) -> El {
    el.radius(2.0)
        .border(1.0, accent.alpha(0.55))
        .shadow(0.0, 10.0, accent.alpha(0.22))
        .press(accent.alpha(0.16))
        .on_press_down(move |state: &mut super::State, _| state.sound(crate::interaction_audio::Cue::KeyDown(false)))
}

/// A molded key standing on a hard 2 point `lip`: it sinks onto the lip while pressed and sounds
/// its down-stroke as the pointer lands (deeper for the `primary` key), its activation the return.
pub(super) fn on_lip(el: El, lip: Color, primary: bool) -> El {
    el.shadow(2.0, 0.0, lip)
        .press_travel(2.0)
        .on_press_down(move |state: &mut super::State, _| state.sound(crate::interaction_audio::Cue::KeyDown(primary)))
}

/// A screen's root on a theme's material.
pub(super) trait Root {
    /// On a molded theme, the dot ground under everything and its keyboard focus ring: 2 points
    /// in the key colour, 3 points out from the control.
    fn material(self, p: &Palette) -> Self;
}

impl Root for El {
    fn material(self, p: &Palette) -> Self {
        let el = self.children(super::ground::ground(p));
        if p.molded {
            el.focus_ring_default(2.0, 3.0, p.accent)
        } else if p.neon {
            el.focus_ring_default(1.5, 2.0, p.accent)
        } else {
            el
        }
    }
}

/// A text field's caret on a molded theme.
pub(super) trait MoldedCaret {
    /// On a molded theme, a block caret in the key colour with the character under it in the
    /// key's legend colour; a flat theme keeps the field's own caret.
    fn molded_caret(self, p: &Palette) -> Self;
}

impl MoldedCaret for El {
    fn molded_caret(self, p: &Palette) -> Self {
        if p.molded || p.neon { self.caret(p.accent, p.selection).caret_block(p.on_accent) } else { self }
    }
}

/// A module: on a molded theme it shades from a lighter top to its metal with a lit top edge;
/// on a flat theme it is the plain page fill.
pub(super) fn module(el: El, p: &Palette) -> El {
    if p.neon {
        el.fill(p.page).border(1.0, p.accent.alpha(0.3)).shadow(0.0, 24.0, p.accent.alpha(0.1))
    } else if p.molded {
        el.gradient(p.hover, p.page).border_gradient(1.0, p.highlight, p.line_strong)
    } else {
        el.fill(p.page).border(1.0, p.line)
    }
}

/// A molded readout's gray LCD: a recessed window of reflective glass, darker at its top edge,
/// lit by the room rather than from behind, with a light catch under its lower lip.
pub(super) fn lcd(el: El, p: &Palette) -> El {
    el.radius(7.0)
        .linear_gradient(std::f32::consts::PI, [(0.0, p.glass[0]), (0.35, p.glass[1]), (1.0, p.glass[2])])
        .box_shadow(::rune::Shadow { y: 1.0, blur: 3.0, color: Color::BLACK.alpha(0.25), inset: true, ..Default::default() })
        .border(0.5, Color::BLACK.alpha(0.15))
        .shadow(1.0, 0.0, Color::WHITE)
}

/// A lamp: a small round light in `color`, glowing on a molded theme.
pub(super) fn led(color: Color, p: &Palette) -> El {
    let lamp = ::rune::el().size(6.0, 6.0).radius(3.0).fill(color).a11y_hidden();
    if p.molded || p.neon { lamp.shadow(0.0, 6.0, color.alpha(0.6)) } else { lamp }
}
