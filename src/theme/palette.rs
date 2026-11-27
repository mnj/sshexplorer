//! The palette: the raw values of the Paper look, private to `theme`. Nothing outside this
//! module's parent names a value; [`Tokens`](super::Tokens) gives each one its job.
//!
//! The values started from REDA's Paper palette and were since made neutral: a near-white
//! window under white cards in light, a near-black one under raised cards in dark.

/// A colour laid over what is underneath: a hex at an opacity in percent.
#[derive(Clone, Copy)]
pub(super) struct Tint {
    pub hex: u32,
    pub percent: f32,
}

/// One hue (an accent preset or a semantic colour) in its strengths.
#[derive(Clone, Copy)]
pub(super) struct Hue {
    pub text: u32,
    pub tint: Tint,
    pub solid: u32,
    pub on_solid: u32,
}

/// One accent preset: its hue, and the focus ring drawn in it.
#[derive(Clone, Copy)]
pub(super) struct AccentHue {
    pub hue: Hue,
    pub ring: u32,
}

/// A shadow: its vertical offset, its blur and the alpha of its black, of 255.
#[derive(Clone, Copy)]
pub(super) struct Soft {
    pub y: i8,
    pub blur: u8,
    pub alpha: u8,
}

/// The raw values of one theme.
pub(super) struct Palette {
    /// The neutral papers, from the window's base up to what floats, and the well.
    pub paper: [u32; 3],
    pub well: u32,
    /// The alpha of the black laid over the window behind a modal, of 255.
    pub scrim_alpha: u8,
    pub ink: u32,
    pub ink_soft: u32,
    pub line: u32,
    pub line_strong: u32,
    /// The ink's opacity, in percent, as a control's fill, on hover, on press and on a
    /// selected row whose selection is not live.
    pub ink_fills: [f32; 4],
    /// The accent presets, in the order of `Accent::ALL`.
    pub accents: [AccentHue; 7],
    pub red: Hue,
    pub amber: Hue,
    pub green: Hue,
    /// The shadows under a card, a popup and a window.
    pub shadows: [Soft; 3],
}

const WHITE: u32 = 0xffffff;

const fn tint(hex: u32, percent: f32) -> Tint {
    Tint { hex, percent }
}

const fn accent(text: u32, tint: Tint, solid: u32, ring: u32) -> AccentHue {
    AccentHue {
        hue: Hue {
            text,
            tint,
            solid,
            on_solid: WHITE,
        },
        ring,
    }
}

const fn soft(y: i8, blur: u8, alpha: u8) -> Soft {
    Soft { y, blur, alpha }
}

pub(super) const LIGHT: Palette = Palette {
    paper: [0xf3f3f1, 0xffffff, 0xffffff],
    well: 0xffffff,
    scrim_alpha: 96,
    ink: 0x18181b,
    ink_soft: 0x5a5a62,
    line: 0xdfdfdb,
    line_strong: 0xc6c6c2,
    ink_fills: [6.0, 5.0, 10.0, 7.0],
    // In light, the ring is the solid; in dark, the text strength.
    accents: [
        accent(0x4b505a, tint(0x5f6673, 14.0), 0x4b505a, 0x4b505a),
        accent(0x1b55ad, tint(0x1f5fbf, 14.0), 0x1f5fbf, 0x1f5fbf),
        accent(0x0f5e63, tint(0x14808a, 14.0), 0x0f6e75, 0x0f6e75),
        accent(0x2f6b1f, tint(0x4c9a2a, 16.0), 0x3a7a25, 0x3a7a25),
        accent(0x5b3fb0, tint(0x6d4fd0, 13.0), 0x6142c4, 0x6142c4),
        accent(0x9a4100, tint(0xe06d10, 16.0), 0xb84e00, 0xb84e00),
        accent(0xa52a6a, tint(0xd0408a, 13.0), 0xb8307a, 0xb8307a),
    ],
    red: Hue {
        text: 0xa8231b,
        tint: tint(0xc8372d, 13.0),
        solid: 0xb3261e,
        on_solid: WHITE,
    },
    amber: Hue {
        text: 0x7a5200,
        tint: tint(0xd99a00, 20.0),
        solid: 0xf0b429,
        on_solid: 0x2a1f00,
    },
    green: Hue {
        text: 0x1f6139,
        tint: tint(0x2e8b57, 15.0),
        solid: 0x26734a,
        on_solid: WHITE,
    },
    shadows: [soft(1, 3, 14), soft(8, 28, 30), soft(16, 48, 40)],
};

pub(super) const DARK: Palette = Palette {
    paper: [0x111113, 0x1a1a1d, 0x26262b],
    well: 0x0e0e10,
    scrim_alpha: 150,
    ink: 0xf2f2f3,
    ink_soft: 0xbcbcc3,
    line: 0x38383e,
    line_strong: 0x4c4c54,
    ink_fills: [8.0, 7.0, 13.0, 10.0],
    accents: [
        accent(0xb9c0cc, tint(0x8a94a6, 22.0), 0x4f5765, 0xb9c0cc),
        accent(0x8fbaf8, tint(0x4f8fe8, 22.0), 0x2458b0, 0x8fbaf8),
        accent(0x6fcfd3, tint(0x3fb0b8, 20.0), 0x146d72, 0x6fcfd3),
        accent(0x9bd27a, tint(0x6cb44a, 18.0), 0x36702a, 0x9bd27a),
        accent(0xbfa8f7, tint(0x8d6ff0, 22.0), 0x5e40b8, 0xbfa8f7),
        accent(0xf5a66a, tint(0xe57a2a, 18.0), 0xa84a0a, 0xf5a66a),
        accent(0xf59ac6, tint(0xe0559a, 20.0), 0xa8326e, 0xf59ac6),
    ],
    red: Hue {
        text: 0xf5998c,
        tint: tint(0xe5534a, 20.0),
        solid: 0xad2f27,
        on_solid: WHITE,
    },
    amber: Hue {
        text: 0xe5b04a,
        tint: tint(0xe5a530, 16.0),
        solid: 0xe5b04a,
        on_solid: 0x231a05,
    },
    green: Hue {
        text: 0x7cc792,
        tint: tint(0x4faf6e, 18.0),
        solid: 0x25683f,
        on_solid: WHITE,
    },
    shadows: [soft(1, 4, 70), soft(6, 24, 100), soft(12, 40, 120)],
};
