//! Cairo drawing for the native backend.
//!
//! Everything here works in logical pixels; the caller applies the output scale to the
//! context before calling in, so nothing below needs to know about HiDPI.

use crate::theme::{Rgb, Theme};
use cairo::{Context, Operator};

/// The card's size in logical pixels, matching the GTK build.
pub const CARD_W: f64 = 520.0;
pub const CARD_H: f64 = 440.0;
const CARD_RADIUS: f64 = 12.0;

/// How far the border and hover tints are pushed away from the card background. These are
/// the alphas the GTK stylesheet wrote as `alpha(@theme_fg_color, ...)`.
const BORDER_ALPHA: f64 = 0.15;

/// Where the card sits inside a full-output surface: centred.
pub fn card_origin(surface_w: f64, surface_h: f64) -> (f64, f64) {
    (
        ((surface_w - CARD_W) / 2.0).floor().max(0.0),
        ((surface_h - CARD_H) / 2.0).floor().max(0.0),
    )
}

/// Paint one frame into `cr`, which covers the whole surface in logical pixels.
///
/// The area outside the card is left fully transparent so the desktop shows through and
/// clicks there read as "dismiss".
pub fn frame(cr: &Context, theme: &Theme, surface_w: f64, surface_h: f64) {
    // Slots come back from the pool holding the previous frame; Source rather than Over so
    // the transparent background actually replaces it instead of compositing onto it.
    cr.set_operator(Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint().unwrap();
    cr.set_operator(Operator::Over);

    let (x, y) = card_origin(surface_w, surface_h);
    cr.save().unwrap();
    cr.translate(x, y);
    card(cr, theme);
    cr.restore().unwrap();
}

/// The card itself: filled rounded rectangle with a hairline border.
fn card(cr: &Context, theme: &Theme) {
    rounded_rect(cr, 0.0, 0.0, CARD_W, CARD_H, CARD_RADIUS);
    set_source(cr, theme.window_bg);
    cr.fill_preserve().unwrap();

    // The border is the foreground colour at low alpha, pre-blended over the card so it
    // needs no second compositing pass.
    set_source(cr, theme.window_fg.blend(theme.window_bg, BORDER_ALPHA));
    cr.set_line_width(1.0);
    cr.stroke().unwrap();
}

pub fn set_source(cr: &Context, c: Rgb) {
    cr.set_source_rgb(c.r, c.g, c.b);
}

/// A rounded rectangle path. Cairo has no primitive for it, and the grid needs the same
/// shape for selection and hover, so it lives here rather than inline.
pub fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    // Clamp so a radius wider than the box cannot invert the arcs.
    let r = r.min(w / 2.0).min(h / 2.0);
    const DEG: f64 = std::f64::consts::PI / 180.0;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -90.0 * DEG, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, 90.0 * DEG);
    cr.arc(x + r, y + h - r, r, 90.0 * DEG, 180.0 * DEG);
    cr.arc(x + r, y + r, r, 180.0 * DEG, 270.0 * DEG);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_is_centred() {
        let (x, y) = card_origin(1920.0, 1200.0);
        assert_eq!(x, (1920.0 - CARD_W) / 2.0);
        assert_eq!(y, (1200.0 - CARD_H) / 2.0);
    }

    #[test]
    fn an_output_smaller_than_the_card_does_not_go_negative() {
        let (x, y) = card_origin(320.0, 200.0);
        assert_eq!((x, y), (0.0, 0.0));
    }
}
