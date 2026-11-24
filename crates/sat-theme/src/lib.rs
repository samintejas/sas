//! The Dots design tokens as Rust values.
//!
//! Everything here is generated at build time from `assets/dots/tokens.json`,
//! the same file the design prototype uses, so the app and the prototype
//! share one source of truth.

#![warn(missing_docs)]

/// An sRGB colour with straight alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha.
    pub a: u8,
}

impl Rgba {
    /// A colour from components.
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// An opaque colour from `0xRRGGBB`.
    pub const fn hex(v: u32) -> Rgba {
        Rgba::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }

    /// The same colour with alpha scaled by `k` (0–1).
    pub fn alpha(self, k: f32) -> Rgba {
        Rgba { a: (self.a as f32 * k.clamp(0.0, 1.0)).round() as u8, ..self }
    }

    /// `k` of this colour over `base` (like CSS `color-mix(in srgb, self k, base)`), opaque.
    pub fn mix(self, base: Rgba, k: f32) -> Rgba {
        let k = k.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 * k + b as f32 * (1.0 - k)).round() as u8;
        Rgba::new(m(self.r, base.r), m(self.g, base.g), m(self.b, base.b), 255)
    }
}

include!(concat!(env!("OUT_DIR"), "/tokens.rs"));

/// The app's theme: a palette plus the track colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Colour tokens.
    pub c: Palette,
    /// Track colours, indexed by a track's colour number (wrapping).
    pub tracks: [Rgba; 8],
}

impl Theme {
    /// The dark theme the prototype uses.
    pub const fn dark() -> Theme {
        Theme { c: DARK, tracks: track_colors(&DARK) }
    }

    /// The light theme.
    pub const fn light() -> Theme {
        Theme { c: LIGHT, tracks: track_colors(&LIGHT) }
    }

    /// The colour for track colour number `n`.
    pub fn track(&self, n: u8) -> Rgba {
        self.tracks[n as usize % self.tracks.len()]
    }
}

const fn track_colors(p: &Palette) -> [Rgba; 8] {
    // the six categorical colours, then the two extra hues the prototype uses
    [p.cat_1, p.cat_2, p.cat_3, p.cat_4, p.cat_5, p.cat_6, Rgba::hex(0xe07a3a), Rgba::hex(0xb36ad6)]
}

/// Font family for labels.
pub const FONT_SANS: &str = "Geist";
/// Font family for numbers and the dense DAW surface.
pub const FONT_MONO: &str = "Geist Mono";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_resolve() {
        assert_eq!(DARK.bg_base, Rgba::hex(0x080808));
        assert_eq!(DARK.accent_primary, Rgba::hex(0xf0a0dc));
        // references like {neutral-8} resolve
        assert_eq!(DARK.text_secondary, DARK.neutral_8);
        assert_eq!(DARK.border_subtle.a, 18);
        assert_eq!(space::SPACE_8, 8.0);
        assert_eq!(radius::RADIUS_MD, 5.0);
        assert_eq!(font_size::FONT_SIZE_XS, 12.0);
        assert_eq!(Theme::dark().track(0), DARK.cat_1);
    }
}
