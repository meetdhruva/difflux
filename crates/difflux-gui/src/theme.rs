//! Colors and type, as design tokens.
//!
//! Diff colors follow the Okabe–Ito palette: vermillion for the left side
//! (removed) and blue for the right side (added) stay distinct under the common
//! color-vision deficiencies, and every state also has a glyph so color is
//! never the only signal. Tints are kept light enough for 4.5:1 text contrast.

use egui::{Color32, Visuals};

/// Semantic colors for one theme.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Palette {
    pub removed_bg: Color32,
    pub removed_strong: Color32,
    pub added_bg: Color32,
    pub added_strong: Color32,
    pub changed_bg: Color32,
    pub filler_bg: Color32,
    pub removed_fg: Color32,
    pub added_fg: Color32,
    pub changed_fg: Color32,
    pub error_fg: Color32,
    pub muted_fg: Color32,
    pub text_fg: Color32,
    pub gutter_bg: Color32,
    pub current_hunk: Color32,
    pub selection_bg: Color32,
    pub ruler_bg: Color32,
    pub ruler_view: Color32,
}

impl Palette {
    pub(crate) fn for_visuals(v: &Visuals) -> Self {
        if v.dark_mode { Self::dark() } else { Self::light() }
    }

    fn dark() -> Self {
        Self {
            removed_bg: Color32::from_rgb(0x4a, 0x22, 0x14),
            removed_strong: Color32::from_rgb(0x8a, 0x3a, 0x12),
            added_bg: Color32::from_rgb(0x12, 0x2f, 0x4a),
            added_strong: Color32::from_rgb(0x1a, 0x55, 0x8c),
            changed_bg: Color32::from_rgb(0x36, 0x30, 0x1a),
            filler_bg: Color32::from_rgb(0x22, 0x22, 0x24),
            removed_fg: Color32::from_rgb(0xff, 0x9e, 0x6b),
            added_fg: Color32::from_rgb(0x7c, 0xc4, 0xf2),
            changed_fg: Color32::from_rgb(0xf0, 0xc0, 0x5a),
            error_fg: Color32::from_rgb(0xff, 0x7a, 0x7a),
            muted_fg: Color32::from_rgb(0x8c, 0x8c, 0x94),
            text_fg: Color32::from_rgb(0xe4, 0xe4, 0xe8),
            gutter_bg: Color32::from_rgb(0x1c, 0x1c, 0x1f),
            current_hunk: Color32::from_rgb(0xe6, 0x9f, 0x00),
            selection_bg: Color32::from_rgb(0x2c, 0x3e, 0x58),
            ruler_bg: Color32::from_rgb(0x1a, 0x1a, 0x1d),
            ruler_view: Color32::from_white_alpha(28),
        }
    }

    fn light() -> Self {
        Self {
            removed_bg: Color32::from_rgb(0xfd, 0xe4, 0xd8),
            removed_strong: Color32::from_rgb(0xf6, 0xb1, 0x8e),
            added_bg: Color32::from_rgb(0xdd, 0xec, 0xfa),
            added_strong: Color32::from_rgb(0x9c, 0xc8, 0xef),
            changed_bg: Color32::from_rgb(0xfa, 0xf1, 0xd6),
            filler_bg: Color32::from_rgb(0xf1, 0xf1, 0xf3),
            removed_fg: Color32::from_rgb(0xa8, 0x3c, 0x00),
            added_fg: Color32::from_rgb(0x00, 0x5a, 0x9c),
            changed_fg: Color32::from_rgb(0x8a, 0x5d, 0x00),
            error_fg: Color32::from_rgb(0xb0, 0x1e, 0x1e),
            muted_fg: Color32::from_rgb(0x6b, 0x6b, 0x73),
            text_fg: Color32::from_rgb(0x1e, 0x1e, 0x22),
            gutter_bg: Color32::from_rgb(0xf6, 0xf6, 0xf8),
            current_hunk: Color32::from_rgb(0xc7, 0x7c, 0x00),
            selection_bg: Color32::from_rgb(0xcf, 0xe0, 0xf5),
            ruler_bg: Color32::from_rgb(0xec, 0xec, 0xef),
            ruler_view: Color32::from_black_alpha(30),
        }
    }
}

/// Theme preference stored in settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub(crate) fn next(self, currently_dark: bool) -> Self {
        match self {
            Self::System => {
                if currently_dark {
                    Self::Light
                } else {
                    Self::Dark
                }
            }
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    pub(crate) fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_on_every_tint_meets_wcag_aa() {
        for p in [Palette::dark(), Palette::light()] {
            for bg in [p.removed_bg, p.removed_strong, p.added_bg, p.added_strong, p.changed_bg, p.filler_bg, p.selection_bg, p.gutter_bg] {
                let ratio = contrast(p.text_fg, bg);
                assert!(ratio >= 4.5, "text contrast {ratio:.2} on {bg:?}");
            }
            for fg in [p.removed_fg, p.added_fg, p.changed_fg, p.error_fg, p.muted_fg] {
                let ratio = contrast(fg, p.gutter_bg);
                assert!(ratio >= 4.5, "accent contrast {ratio:.2} for {fg:?}");
            }
        }
    }
}
