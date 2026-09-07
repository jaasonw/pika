//! Desktop colours for the native backend.
//!
//! Read straight from `~/.config/kdeglobals`. That is a plain file read - a few hundred
//! microseconds - where the `org.freedesktop.appearance` portal costs a D-Bus round trip on
//! a path whose whole budget is tens of milliseconds, and the portal only reports a
//! light/dark preference and an accent colour anyway. On the desktop this picker targets,
//! kdeglobals is both cheaper and more faithful. See the note in the plan about what the
//! portal would still be good for on non-KDE desktops.

/// A colour in Cairo's units: components in `[0.0, 1.0]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    const fn from_u8(r: u8, g: u8, b: u8) -> Self {
        Rgb {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
        }
    }

    /// Perceived brightness. The coefficients are the usual Rec. 601 luma weights.
    /// Only the tests read this today; it is the natural place for a light/dark decision
    /// if one is ever needed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn luma(&self) -> f64 {
        0.299 * self.r + 0.587 * self.g + 0.114 * self.b
    }

    /// This colour blended over `under` at `alpha`. The card is opaque by the time these
    /// are drawn, so pre-blending here avoids a second Cairo group per hover cell.
    pub fn blend(&self, under: Rgb, alpha: f64) -> Rgb {
        Rgb {
            r: under.r + (self.r - under.r) * alpha,
            g: under.g + (self.g - under.g) * alpha,
            b: under.b + (self.b - under.b) * alpha,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// The card background.
    pub window_bg: Rgb,
    /// Body text on `window_bg`.
    pub window_fg: Rgb,
    /// The search field's background, which KDE themes a shade apart from the window.
    pub view_bg: Rgb,
    /// Behind the selected cell.
    pub selection_bg: Rgb,
    /// The selected cell's glyph colour.
    pub selection_fg: Rgb,
}

/// Breeze Light, KDE's shipped default, used when there is no kdeglobals to read.
const BREEZE_LIGHT: Theme = Theme {
    window_bg: Rgb::from_u8(239, 240, 241),
    window_fg: Rgb::from_u8(35, 38, 41),
    view_bg: Rgb::from_u8(252, 252, 252),
    selection_bg: Rgb::from_u8(61, 174, 233),
    selection_fg: Rgb::from_u8(252, 252, 252),
};

impl Theme {
    pub fn load() -> Theme {
        dirs::config_dir()
            .map(|d| d.join("kdeglobals"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| Theme::parse(&s))
            .unwrap_or(BREEZE_LIGHT)
    }

    /// Pull the five colours we draw with out of a kdeglobals body.
    ///
    /// Returns `None` unless at least the window pair was found, so a truncated or
    /// unrelated file falls back to Breeze rather than to a half-filled palette.
    fn parse(src: &str) -> Option<Theme> {
        let mut section = "";
        let (mut win_bg, mut win_fg) = (None, None);
        let (mut sel_bg, mut sel_fg) = (None, None);
        let mut view_bg = None;

        for line in src.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = match name {
                    "Colors:Window" => "win",
                    "Colors:Selection" => "sel",
                    "Colors:View" => "view",
                    // Any other section: keep scanning, but ignore its keys.
                    _ => "",
                };
                continue;
            }
            if section.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let slot = match (section, key.trim()) {
                ("win", "BackgroundNormal") => &mut win_bg,
                ("win", "ForegroundNormal") => &mut win_fg,
                ("sel", "BackgroundNormal") => &mut sel_bg,
                ("sel", "ForegroundNormal") => &mut sel_fg,
                ("view", "BackgroundNormal") => &mut view_bg,
                _ => continue,
            };
            *slot = parse_rgb(value);
        }

        let window_bg = win_bg?;
        let window_fg = win_fg?;
        Some(Theme {
            window_bg,
            window_fg,
            // The remaining three are cosmetic; Breeze's are a reasonable stand-in if a
            // partial colour scheme omits them.
            view_bg: view_bg.unwrap_or(BREEZE_LIGHT.view_bg),
            selection_bg: sel_bg.unwrap_or(BREEZE_LIGHT.selection_bg),
            selection_fg: sel_fg.unwrap_or(window_bg),
        })
    }
}

/// `"203,166,247"` to an [`Rgb`]. Anything else is `None`.
fn parse_rgb(value: &str) -> Option<Rgb> {
    let mut parts = value.trim().splitn(3, ',');
    let mut next = || parts.next()?.trim().parse::<u8>().ok();
    let (r, g, b) = (next()?, next()?, next()?);
    Some(Rgb::from_u8(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[ColorEffects:Disabled]
ColorAmount=0

[Colors:Selection]
BackgroundAlternate=69,71,90
BackgroundNormal=203,166,247
ForegroundNormal=30,30,46

[Colors:View]
BackgroundNormal=34,34,51

[Colors:Window]
BackgroundAlternate=24,24,37
BackgroundNormal=30,30,46
ForegroundNormal=205,214,244

[General]
fixed=Monospace,10
";

    #[test]
    fn reads_the_sections_it_needs_and_skips_the_rest() {
        let t = Theme::parse(SAMPLE).expect("window pair present");
        assert_eq!(t.window_bg, Rgb::from_u8(30, 30, 46));
        assert_eq!(t.window_fg, Rgb::from_u8(205, 214, 244));
        assert_eq!(t.view_bg, Rgb::from_u8(34, 34, 51));
        assert_eq!(t.selection_bg, Rgb::from_u8(203, 166, 247));
        assert_eq!(t.selection_fg, Rgb::from_u8(30, 30, 46));
        assert!(t.window_bg.luma() < 0.5, "the sample scheme is a dark one");
    }

    #[test]
    fn a_file_without_the_window_colours_falls_back() {
        assert!(Theme::parse("[General]\nfixed=Monospace,10\n").is_none());
        assert!(Theme::parse("").is_none());
    }

    #[test]
    fn a_partial_scheme_keeps_the_colours_it_did_define() {
        let t = Theme::parse("[Colors:Window]\nBackgroundNormal=0,0,0\nForegroundNormal=255,255,255\n")
            .expect("window pair present");
        assert_eq!(t.window_bg, Rgb::from_u8(0, 0, 0));
        // Selection foreground has no sensible Breeze default against an unknown window
        // colour, so it falls back to the window background instead.
        assert_eq!(t.selection_fg, Rgb::from_u8(0, 0, 0));
        assert_eq!(t.selection_bg, BREEZE_LIGHT.selection_bg);
    }

    #[test]
    fn malformed_values_are_ignored_rather_than_panicking() {
        assert_eq!(parse_rgb("1,2"), None);
        assert_eq!(parse_rgb("300,0,0"), None);
        assert_eq!(parse_rgb("not,a,colour"), None);
        assert_eq!(parse_rgb(" 1 , 2 , 3 "), Some(Rgb::from_u8(1, 2, 3)));
    }

    #[test]
    fn blend_moves_toward_the_overlay() {
        let black = Rgb::from_u8(0, 0, 0);
        let white = Rgb::from_u8(255, 255, 255);
        assert_eq!(white.blend(black, 0.0), black);
        assert_eq!(white.blend(black, 1.0), white);
        assert!((white.blend(black, 0.5).luma() - 0.5).abs() < 1e-9);
    }
}
