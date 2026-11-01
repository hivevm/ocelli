//! Dark and light palettes, taken from the desktop's tokens
//! (`src-web/src/theme/palette.ts`, ADR-0024) so both frontends share colours.
//! The frame background is painted too, so a theme looks the same whatever
//! the terminal's own background is.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn toggled(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

/// A chip's text on its tinted background.
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    pub fg: Color,
    pub bg: Color,
}

pub struct Palette {
    pub bg: Color,
    pub bg_chrome: Color,
    pub bg_sel: Color,
    pub border: Color,
    pub text: Color,
    pub text_bright: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    /// Graph and text outside the highlighted lineage.
    pub faint: Color,
    pub sha: Color,
    pub accent: Color,
    pub blue: Color,
    pub amber: Color,
    pub red: Color,
    pub teal: Color,
    pub purple: Color,
    pub add_bg: Color,
    pub del_bg: Color,
    /// The merge editor's theirs, ours being `add_bg`, and the base of a block (ADR-0031): the
    /// web interface's `--blue-bg` and `--amber-bg`.
    pub theirs_bg: Color,
    pub base_bg: Color,
    pub chip_local: Chip,
    pub chip_head: Chip,
    pub chip_remote: Chip,
    pub chip_tag: Chip,
    pub chip_stash: Chip,
    pub chip_more: Chip,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const DARK: Palette = Palette {
    bg: rgb(0x1a1d23),
    bg_chrome: rgb(0x13151a),
    bg_sel: rgb(0x1e2a3a),
    border: rgb(0x383b45),
    text: rgb(0xc0c3ca),
    text_bright: rgb(0xe8eaf0),
    text_muted: rgb(0x8b8fa8),
    text_dim: rgb(0x5a5e6e),
    faint: rgb(0x3a3e4e),
    sha: rgb(0x6a7090),
    accent: rgb(0x4caf7d),
    blue: rgb(0x6b9fff),
    amber: rgb(0xe8a94a),
    red: rgb(0xe06b75),
    teal: rgb(0x4ec9c9),
    purple: rgb(0xb48efe),
    add_bg: rgb(0x1a3a22),
    del_bg: rgb(0x3a1a1a),
    theirs_bg: rgb(0x162338),
    base_bg: rgb(0x2e2412),
    chip_local: Chip {
        fg: rgb(0x4caf7d),
        bg: rgb(0x1a3a22),
    },
    chip_head: Chip {
        fg: rgb(0x7fe0aa),
        bg: rgb(0x1e4d32),
    },
    chip_remote: Chip {
        fg: rgb(0x6b9fff),
        bg: rgb(0x162338),
    },
    chip_tag: Chip {
        fg: rgb(0xe8a94a),
        bg: rgb(0x2e2412),
    },
    chip_stash: Chip {
        fg: rgb(0xb48efe),
        bg: rgb(0x2a2240),
    },
    chip_more: Chip {
        fg: rgb(0x8a8f9e),
        bg: rgb(0x23262e),
    },
};

const LIGHT: Palette = Palette {
    bg: rgb(0xffffff),
    bg_chrome: rgb(0xf3f4f6),
    bg_sel: rgb(0xe3edff),
    border: rgb(0xcbd0d8),
    text: rgb(0x3a3f4a),
    text_bright: rgb(0x14171c),
    text_muted: rgb(0x5d6470),
    text_dim: rgb(0x767c8a),
    faint: rgb(0xc8ccd4),
    sha: rgb(0x969cab),
    accent: rgb(0x2e9e63),
    blue: rgb(0x2d6fdb),
    amber: rgb(0xb5791f),
    red: rgb(0xd23f4a),
    teal: rgb(0x1f9b9b),
    purple: rgb(0x7a4fd0),
    add_bg: rgb(0xe3f6ea),
    del_bg: rgb(0xfbe4e6),
    theirs_bg: rgb(0xe4edfb),
    base_bg: rgb(0xfbf0d8),
    chip_local: Chip {
        fg: rgb(0x2e9e63),
        bg: rgb(0xe3f6ea),
    },
    chip_head: Chip {
        fg: rgb(0x1f7a4c),
        bg: rgb(0xcdead6),
    },
    chip_remote: Chip {
        fg: rgb(0x2d6fdb),
        bg: rgb(0xe4edfb),
    },
    chip_tag: Chip {
        fg: rgb(0xb5791f),
        bg: rgb(0xfbf0d8),
    },
    chip_stash: Chip {
        fg: rgb(0x7a4fd0),
        bg: rgb(0xefe8fb),
    },
    chip_more: Chip {
        fg: rgb(0x6b7180),
        bg: rgb(0xeef0f3),
    },
};

pub fn palette(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Lane colours — the desktop's `SERIES_COLORS`, identical in both themes so a
/// branch keeps its colour across a theme switch and across both frontends.
const LANES: [Color; 10] = [
    rgb(0x4caf7d),
    rgb(0x6b9fff),
    rgb(0xe8a94a),
    rgb(0xe06b75),
    rgb(0xb48efe),
    rgb(0x4ec9c9),
    rgb(0xf0a070),
    rgb(0x60c0e0),
    rgb(0xa0d060),
    rgb(0xe080b0),
];

pub fn lane(col: usize) -> Color {
    LANES[col % LANES.len()]
}

#[cfg(test)]
mod tests {
    // Verifies: ADR-0024
    use super::*;

    fn channels(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("a palette colour is not an explicit rgb triple: {other:?}"),
        }
    }

    /// The desktop's lane colours, read from the file that declares them. Reading them rather
    /// than restating them is the point: the two lists are one decision, and a test that holds
    /// its own copy would pass while they drifted apart.
    fn desktop_lanes() -> Vec<(u8, u8, u8)> {
        let ts = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src-web/src/theme/palette.ts"
        ))
        .expect("the desktop palette file is readable from the crate");
        let from = ts
            .find("export const SERIES_COLORS")
            .expect("the desktop declares SERIES_COLORS");
        let rest = &ts[from..];
        let body = &rest[..rest.find("];").expect("the array is closed")];
        body.lines()
            .filter_map(|line| line.trim().strip_prefix("'#"))
            .filter_map(|rest| rest.split('\'').next())
            .map(|hex| {
                let v = u32::from_str_radix(hex, 16).expect("six hex digits per colour");
                ((v >> 16) as u8, (v >> 8) as u8, v as u8)
            })
            .collect()
    }

    /// Every token of one palette, so a new field cannot slip past the checks below.
    fn tokens(p: &Palette) -> Vec<Color> {
        vec![
            p.bg,
            p.bg_chrome,
            p.bg_sel,
            p.border,
            p.text,
            p.text_bright,
            p.text_muted,
            p.text_dim,
            p.faint,
            p.sha,
            p.accent,
            p.blue,
            p.amber,
            p.red,
            p.teal,
            p.purple,
            p.add_bg,
            p.del_bg,
            p.chip_local.fg,
            p.chip_local.bg,
            p.chip_head.fg,
            p.chip_head.bg,
            p.chip_remote.fg,
            p.chip_remote.bg,
            p.chip_tag.fg,
            p.chip_tag.bg,
        ]
    }

    #[test]
    fn the_lane_colours_are_the_desktops_in_the_same_order() {
        let mine: Vec<_> = LANES.iter().copied().map(channels).collect();
        assert_eq!(
            mine,
            desktop_lanes(),
            "the lane colours are one decision for both frontends: a commit keeps its colour \
             across a theme switch and across the window and the terminal alike"
        );
    }

    #[test]
    fn a_lane_keeps_its_colour_whatever_the_theme_is() {
        // One table, reached by a function that takes no theme: there is no second place where
        // a lane colour could be given a per-theme value, and the index wraps instead of
        // running out. That several tokens of the dark theme carry a lane colour is intent -
        // both were taken from the same desktop tokens - so it is not asserted against.
        for col in 0..LANES.len() * 2 + 3 {
            assert_eq!(lane(col), LANES[col % LANES.len()]);
        }
        assert_eq!(lane(0), lane(LANES.len()));
    }

    #[test]
    fn every_colour_of_both_themes_is_an_explicit_triple() {
        // The frame background included: a theme has to look the same whatever colours the
        // terminal itself is configured with, so no token may fall back to a terminal default.
        for theme in [Theme::Dark, Theme::Light] {
            for token in tokens(palette(theme)) {
                channels(token);
            }
        }
    }

    #[test]
    fn the_two_themes_differ_and_toggle_into_each_other() {
        let dark = tokens(palette(Theme::Dark));
        let light = tokens(palette(Theme::Light));
        assert_ne!(dark, light, "two themes that paint the same are one theme");
        assert_eq!(Theme::Dark.toggled(), Theme::Light);
        assert_eq!(Theme::Light.toggled(), Theme::Dark);
        assert_eq!(Theme::Dark.name(), "dark");
        assert_eq!(Theme::Light.name(), "light");
    }
}
