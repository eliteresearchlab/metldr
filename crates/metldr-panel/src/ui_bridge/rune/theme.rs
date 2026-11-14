//! The one source of colour for every Rune surface (docs: design/system.md).
//!
//! Values follow beautiful-ui's `app/globals.css`; components read roles from here and never
//! write colour literals of their own.

use ::rune::{Color, oklch};

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
}

type Lch = (f32, f32, f32);

/// One theme's roles as oklch triples, in the order of [`Palette`]'s first fifteen fields.
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
};

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
};

impl Palette {
    pub(super) fn for_theme(theme: &str) -> Self {
        let ramp = match theme {
            "light" => &LIGHT,
            "gruvbox" => &GRUVBOX,
            "cyberpunk" => &CYBERPUNK,
            _ => &DARK,
        };
        let c = |(l, c, h): Lch| oklch(l, c, h);
        let light = std::ptr::eq(ramp, &LIGHT);
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
            accent: c(ramp.accent),
            green: c(ramp.green),
            orange: c(ramp.orange),
            red: c(ramp.red),
            // A neutral overlay that reads on both grounds; darker ink on light themes.
            press: c(ramp.ink).alpha(if light { 0.08 } else { 0.12 }),
            press_solid: c(ramp.surface).alpha(0.16),
            selection: c(ramp.accent).alpha(0.3),
            light,
        }
    }

    /// Distinct series colours for charts, spaced by hue at the accent's lightness and chroma.
    pub(super) fn series(&self, index: usize) -> Color {
        const HUES: [f32; 8] = [254.9, 150.9, 49.9, 300.0, 23.0, 195.0, 85.0, 330.0];
        let l = if self.light { 0.62 } else { 0.72 };
        oklch(l, 0.15, HUES[index % HUES.len()])
    }
}
