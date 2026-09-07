//! Cairo drawing for the native backend.
//!
//! Everything here works in logical pixels; the caller applies the output scale to the
//! context before calling in, so nothing below needs to know about HiDPI.

use crate::emoji;
use crate::grid::{self, Row};
use crate::picker::{self, Mode, Picker, Setting};
use crate::store::Settings;
use crate::theme::{Rgb, Theme};
use cairo::{Context, Operator};
use pango::FontDescription;

/// Padding between the card's edge and its contents.
const PAD: f64 = 12.0;
const SEARCH_H: f64 = 36.0;
const TABS_H: f64 = 30.0;
const FOOTER_H: f64 = 22.0;
/// Gaps between the four stacked regions.
const GAP: f64 = 8.0;
/// Width reserved at the right of the search row for the gear.
const GEAR_W: f64 = 28.0;
/// Height of one settings row, including the gap under it.
const SETTING_ROW_H: f64 = 40.0;
/// One skin-tone swatch in the tone row.
const TONE_SWATCH: f64 = 34.0;
const TONE_COUNT: usize = 6;
/// The toggle switch: track, and the knob inset inside it.
const SWITCH_W: f64 = 42.0;
const SWITCH_H: f64 = 22.0;
const SWITCH_PAD: f64 = 3.0;
/// A stepper button either side of the recents number, and the gap they leave for it.
const STEP_W: f64 = 26.0;
const STEP_VALUE_W: f64 = 40.0;
/// The credit line at the foot of the settings mode. Split so the link can be drawn and
/// hit-tested apart from the text around it.
const CREDIT_HEAD: &str = "created by jasonw | copyright 2026 | open source under mit license | ";
pub const CREDIT_LINK: &str = "donate";
pub const DONATE_URL: &str = "https://ko-fi.com/wayson";

/// Left edge of the search text, and of anything measured against it.
const TEXT_X: f64 = PAD + 10.0;
/// Right edge every row's control is aligned against.
const CONTROL_R: f64 = CARD_W - PAD - 10.0;
/// A focused row gets a bar down its left edge rather than a flooded background.
const FOCUS_BAR_W: f64 = 3.0;
const ROW_FOCUS_ALPHA: f64 = 0.10;
const ROW_HOVER_ALPHA: f64 = 0.09;

/// The card is exactly as wide as twelve cells plus its padding, so the grid never has a
/// ragged right edge.
pub const CARD_W: f64 = grid::COLUMNS as f64 * grid::CELL + PAD * 2.0;
pub const CARD_H: f64 =
    PAD + SEARCH_H + GAP + TABS_H + GAP + picker::VIEWPORT_H + GAP + FOOTER_H + PAD;

const CARD_RADIUS: f64 = 12.0;
const CELL_RADIUS: f64 = 6.0;

const BORDER_ALPHA: f64 = 0.15;
const HOVER_ALPHA: f64 = 0.10;
const HEADER_ALPHA: f64 = 0.55;
const FOOTER_ALPHA: f64 = 0.70;
const PLACEHOLDER_ALPHA: f64 = 0.45;

/// Top of each region, relative to the card.
const SEARCH_Y: f64 = PAD;
const TABS_Y: f64 = SEARCH_Y + SEARCH_H + GAP;
const VIEW_Y: f64 = TABS_Y + TABS_H + GAP;
const FOOTER_Y: f64 = VIEW_Y + picker::VIEWPORT_H + GAP;

/// Where the card sits inside a full-output surface: centred.
pub fn card_origin(surface_w: f64, surface_h: f64) -> (f64, f64) {
    (
        ((surface_w - CARD_W) / 2.0).floor().max(0.0),
        ((surface_h - CARD_H) / 2.0).floor().max(0.0),
    )
}

/// Where the scrolling viewport sits inside a full-output surface, so the backend can turn
/// a pointer position into grid coordinates.
pub fn viewport_origin(surface_w: f64, surface_h: f64) -> (f64, f64) {
    let (x, y) = card_origin(surface_w, surface_h);
    (x + PAD, y + VIEW_Y)
}

/// Fonts are parsed once per frame rather than once per cell: shaping ~100 cells through
/// `FontDescription::from_string` each time showed up as the bulk of a redraw.
struct Fonts {
    ui: FontDescription,
    ui_bold: FontDescription,
    small: FontDescription,
    emoji: FontDescription,
    tab: FontDescription,
}

impl Fonts {
    fn new() -> Fonts {
        let mut ui_bold = FontDescription::from_string("Sans 11");
        ui_bold.set_weight(pango::Weight::Bold);
        Fonts {
            ui: FontDescription::from_string("Sans 11"),
            ui_bold,
            small: FontDescription::from_string("Sans 8"),
            emoji: FontDescription::from_string("Noto Color Emoji 18"),
            tab: FontDescription::from_string("Noto Color Emoji 13"),
        }
    }
}

/// Paint one frame into `cr`, which covers the whole surface in logical pixels.
///
/// The area outside the card is left fully transparent so the desktop shows through and
/// clicks there read as "dismiss".
pub fn frame(
    cr: &Context,
    theme: &Theme,
    p: &Picker,
    set: &Settings,
    surface_w: f64,
    surface_h: f64,
) {
    // Slots come back from the pool holding the previous frame; Source rather than Over so
    // the transparent background actually replaces it instead of compositing onto it.
    cr.set_operator(Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint().unwrap();
    cr.set_operator(Operator::Over);

    let (x, y) = card_origin(surface_w, surface_h);
    let fonts = Fonts::new();
    cr.save().unwrap();
    cr.translate(x, y);
    card(cr, theme);
    match p.mode {
        Mode::Browse => {
            search(cr, theme, &fonts, p);
            gear(cr, theme, &fonts);
            tabs(cr, theme, &fonts, p);
            viewport(cr, theme, &fonts, p);
            footer(cr, theme, &fonts, p);
        }
        Mode::Settings => settings(cr, theme, &fonts, p, set),
    }
    cr.restore().unwrap();
}

/// The gear, top-right of the search row.
fn gear(cr: &Context, theme: &Theme, fonts: &Fonts) {
    let layout = layout_for(cr, &fonts.small, "\u{2699}\u{fe0f}");
    let (tw, th) = layout.pixel_size();
    set_source(cr, theme.window_fg.blend(theme.view_bg, 0.45));
    cr.move_to(
        CARD_W - PAD - GEAR_W + (GEAR_W - tw as f64) / 2.0,
        SEARCH_Y + (SEARCH_H - th as f64) / 2.0,
    );
    pangocairo::functions::show_layout(cr, &layout);
}

/// The settings mode: a title and one row per option, navigated with the arrow keys.
fn settings(cr: &Context, theme: &Theme, fonts: &Fonts, p: &Picker, set: &Settings) {
    set_source(cr, theme.window_fg);
    let layout = layout_for(cr, &fonts.ui_bold, "Pika Settings");
    cr.move_to(PAD + 4.0, SEARCH_Y + 6.0);
    pangocairo::functions::show_layout(cr, &layout);

    let w = CARD_W - PAD * 2.0;
    let h = SETTING_ROW_H - 4.0;
    for (i, row) in picker::SETTINGS.iter().enumerate() {
        let y = setting_y(i);
        let focused = i == p.setting;
        let hovered = p.hover_setting == Some(i);

        if focused || hovered {
            let alpha = if focused { ROW_FOCUS_ALPHA } else { ROW_HOVER_ALPHA };
            rounded_rect(cr, PAD, y, w, h, CELL_RADIUS);
            set_source(cr, theme.window_fg.blend(theme.window_bg, alpha));
            cr.fill().unwrap();
        }
        if focused {
            // The keyboard's position, kept distinct from the pointer's. A bar rather than
            // a flooded row, so every control below keeps one background to sit on.
            rounded_rect(cr, PAD, y + 4.0, FOCUS_BAR_W, h - 8.0, FOCUS_BAR_W / 2.0);
            set_source(cr, theme.selection_bg);
            cr.fill().unwrap();
        }

        set_source(cr, theme.window_fg);
        let layout = layout_for(cr, &fonts.ui, label_for(*row));
        let (_, th) = layout.pixel_size();
        cr.move_to(PAD + 12.0, y + (h - th as f64) / 2.0);
        pangocairo::functions::show_layout(cr, &layout);

        match row {
            Setting::Insert => switch(cr, theme, y, h, set.insert),
            Setting::AlwaysCopy => switch(cr, theme, y, h, set.always_copy),
            Setting::Tone => tone_strip(cr, theme, fonts, y, h, set.skin_tone),
            Setting::RecentLimit => stepper(cr, theme, fonts, y, h, set.recent_limit),
            // The action rows say what they did, once they have done it.
            Setting::ClearRecents if p.cleared_recents => note(cr, theme, fonts, y, h, "Cleared"),
            Setting::ResetPaste if p.reset_paste => {
                note(cr, theme, fonts, y, h, "KDE will ask again")
            }
            _ => {}
        }
    }

    // The key hint sits under the last row; the foot of the card belongs to the credits.
    set_source(cr, theme.window_fg.blend(theme.window_bg, FOOTER_ALPHA));
    let layout = layout_for(
        cr,
        &fonts.small,
        "Click or use the arrows  \u{2022}  Enter activates  \u{2022}  Esc goes back",
    );
    cr.move_to(PAD + 12.0, setting_y(picker::SETTINGS.len()) + 6.0);
    pangocairo::functions::show_layout(cr, &layout);

    credits(cr, theme, fonts);
}

/// Where the credit line starts, and how wide its two halves are. Drawing and hit-testing
/// both need this, and it is measured rather than assumed so the link stays aligned with
/// whatever the UI font actually shapes to.
fn credit_metrics(cr: &Context, fonts: &Fonts) -> (f64, f64, f64) {
    let head = layout_for(cr, &fonts.small, CREDIT_HEAD);
    let link = layout_for(cr, &fonts.small, CREDIT_LINK);
    let (hw, _) = head.pixel_size();
    let (lw, _) = link.pixel_size();
    let (hw, lw) = (hw as f64, lw as f64);
    (((CARD_W - (hw + lw)) / 2.0).max(PAD), hw, lw)
}

fn credits(cr: &Context, theme: &Theme, fonts: &Fonts) {
    let (x, head_w, link_w) = credit_metrics(cr, fonts);

    set_source(cr, theme.window_fg.blend(theme.window_bg, 0.45));
    let layout = layout_for(cr, &fonts.small, CREDIT_HEAD);
    let (_, th) = layout.pixel_size();
    let y = FOOTER_Y + (FOOTER_H - th as f64) / 2.0;
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);

    // The link is the accent colour and underlined, since a bare word in a footer does not
    // read as something to click.
    set_source(cr, theme.selection_bg);
    let layout = layout_for(cr, &fonts.small, CREDIT_LINK);
    cr.move_to(x + head_w, y);
    pangocairo::functions::show_layout(cr, &layout);
    cr.rectangle(x + head_w, y + th as f64 - 1.0, link_w, 1.0);
    cr.fill().unwrap();
}

/// A toggle switch: a filled track with the knob at the end matching its state.
fn switch(cr: &Context, theme: &Theme, y: f64, row_h: f64, on: bool) {
    let x = switch_x();
    let top = y + (row_h - SWITCH_H) / 2.0;
    rounded_rect(cr, x, top, SWITCH_W, SWITCH_H, SWITCH_H / 2.0);
    if on {
        set_source(cr, theme.selection_bg);
    } else {
        // Off still needs a visible track, or it reads as empty space.
        set_source(cr, theme.window_fg.blend(theme.window_bg, 0.22));
    }
    cr.fill().unwrap();

    let knob = SWITCH_H - SWITCH_PAD * 2.0;
    let knob_x = if on {
        x + SWITCH_W - SWITCH_PAD - knob
    } else {
        x + SWITCH_PAD
    };
    rounded_rect(cr, knob_x, top + SWITCH_PAD, knob, knob, knob / 2.0);
    set_source(cr, if on { theme.selection_fg } else { theme.window_bg });
    cr.fill().unwrap();
}

/// The recents cap, with a button either side so it is clickable, not just keyboard-only.
fn stepper(cr: &Context, theme: &Theme, fonts: &Fonts, y: f64, row_h: f64, value: usize) {
    let x0 = stepper_x();
    let top = y + (row_h - SWITCH_H) / 2.0;
    let track = theme.window_fg.blend(theme.window_bg, 0.14);

    for (i, glyph) in ["\u{2212}", "+"].iter().enumerate() {
        let x = x0 + i as f64 * (STEP_W + STEP_VALUE_W);
        rounded_rect(cr, x, top, STEP_W, SWITCH_H, CELL_RADIUS);
        set_source(cr, track);
        cr.fill().unwrap();

        set_source(cr, theme.window_fg);
        let layout = layout_for(cr, &fonts.ui, glyph);
        let (tw, th) = layout.pixel_size();
        cr.move_to(
            x + (STEP_W - tw as f64) / 2.0,
            top + (SWITCH_H - th as f64) / 2.0,
        );
        pangocairo::functions::show_layout(cr, &layout);
    }

    set_source(cr, theme.window_fg);
    let layout = layout_for(cr, &fonts.ui, &value.to_string());
    let (tw, th) = layout.pixel_size();
    cr.move_to(
        x0 + STEP_W + (STEP_VALUE_W - tw as f64) / 2.0,
        y + (row_h - th as f64) / 2.0,
    );
    pangocairo::functions::show_layout(cr, &layout);
}

/// A dimmed word at the right of a row, for an action reporting what it did.
fn note(cr: &Context, theme: &Theme, fonts: &Fonts, y: f64, row_h: f64, text: &str) {
    set_source(cr, theme.window_fg.blend(theme.window_bg, FOOTER_ALPHA));
    let layout = layout_for(cr, &fonts.small, text);
    let (tw, th) = layout.pixel_size();
    cr.move_to(CONTROL_R - tw as f64, y + (row_h - th as f64) / 2.0);
    pangocairo::functions::show_layout(cr, &layout);
}

/// The six tones as a row of swatches, so the choice is one click rather than a cycle.
fn tone_strip(cr: &Context, theme: &Theme, fonts: &Fonts, y: f64, row_h: f64, tone: u8) {
    let x0 = tone_strip_x();
    for i in 0..TONE_COUNT {
        let x = x0 + i as f64 * TONE_SWATCH;
        let box_y = y + (row_h - TONE_SWATCH) / 2.0;
        if i as u8 == tone {
            rounded_rect(cr, x + 1.0, box_y, TONE_SWATCH - 2.0, TONE_SWATCH, CELL_RADIUS);
            set_source(cr, theme.selection_bg);
            cr.fill().unwrap();
        }
        let wave = emoji::find("\u{270b}").map(|e| e.toned(i as u8)).unwrap_or("\u{270b}");
        let layout = layout_for(cr, &fonts.tab, wave);
        let (tw, th) = layout.pixel_size();
        cr.move_to(
            x + (TONE_SWATCH - tw as f64) / 2.0,
            box_y + (TONE_SWATCH - th as f64) / 2.0,
        );
        pangocairo::functions::show_layout(cr, &layout);
    }
}

fn label_for(row: Setting) -> &'static str {
    match row {
        Setting::Insert => "Insert into the focused field",
        Setting::AlwaysCopy => "Always copy as well",
        Setting::Tone => "Skin tone",
        Setting::RecentLimit => "Recents to remember",
        Setting::ClearRecents => "Clear recents",
        Setting::ResetPaste => "Reset paste permission",
        Setting::Back => "Back to the picker",
    }
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

fn search(cr: &Context, theme: &Theme, fonts: &Fonts, p: &Picker) {
    let w = CARD_W - PAD * 2.0 - GEAR_W;
    rounded_rect(cr, PAD, SEARCH_Y, w, SEARCH_H, CELL_RADIUS);
    set_source(cr, theme.view_bg);
    cr.fill().unwrap();

    let (text, colour) = if p.query.is_empty() {
        (
            "Search emoji",
            theme.window_fg.blend(theme.view_bg, PLACEHOLDER_ALPHA),
        )
    } else {
        (p.query.text(), theme.window_fg)
    };
    set_source(cr, colour);
    let layout = layout_for(cr, &fonts.ui, text);
    let (_, th) = layout.pixel_size();
    cr.move_to(TEXT_X, SEARCH_Y + (SEARCH_H - th as f64) / 2.0);
    pangocairo::functions::show_layout(cr, &layout);

    // The caret sits at the cursor rather than at the end, so word motion and
    // click-to-position are visible. Pango is asked where the byte offset landed instead
    // of the text being measured twice.
    if !p.query.is_empty() {
        let (rect, _) = layout.cursor_pos(p.query.cursor() as i32);
        let x = TEXT_X + rect.x() as f64 / pango::SCALE as f64;
        set_source(cr, theme.window_fg.blend(theme.view_bg, 0.6));
        cr.rectangle(x, SEARCH_Y + 8.0, 1.5, SEARCH_H - 16.0);
        cr.fill().unwrap();
    }
}

fn tabs(cr: &Context, theme: &Theme, fonts: &Fonts, p: &Picker) {
    let active = p.grid.active_section(p.scroll);
    // Recents first, then one per emoji group, matching `Grid::sections`.
    let count = p.grid.sections.len();
    let slot = (CARD_W - PAD * 2.0) / count as f64;

    for i in 0..count {
        let x = PAD + slot * i as f64;
        let enabled = p.grid.sections[i].is_some();
        if Some(i) == active {
            rounded_rect(cr, x + 2.0, TABS_Y, slot - 4.0, TABS_H, CELL_RADIUS);
            set_source(cr, theme.window_fg.blend(theme.window_bg, HOVER_ALPHA));
            cr.fill().unwrap();
        }
        let icon = if i == 0 {
            "\u{1f553}"
        } else {
            grid::group_icon(emoji::GROUPS[i - 1])
        };
        let layout = layout_for(cr, &fonts.tab, icon);
        let (tw, th) = layout.pixel_size();
        // A tab whose section is empty is drawn dimmed rather than hidden, so the bar
        // does not reflow when the recents list fills up.
        set_source(
            cr,
            if enabled {
                theme.window_fg
            } else {
                theme.window_fg.blend(theme.window_bg, 0.35)
            },
        );
        cr.move_to(
            x + (slot - tw as f64) / 2.0,
            TABS_Y + (TABS_H - th as f64) / 2.0,
        );
        pangocairo::functions::show_layout(cr, &layout);
    }
}

/// The scrolling grid. Only rows overlapping the viewport are drawn - at ~160 rows that is
/// the difference between eight rows of work and all of them, every frame.
fn viewport(cr: &Context, theme: &Theme, fonts: &Fonts, p: &Picker) {
    cr.save().unwrap();
    cr.rectangle(PAD, VIEW_Y, CARD_W - PAD * 2.0, picker::VIEWPORT_H);
    cr.clip();

    let first = p.grid.row_at(p.scroll);
    let mut y = VIEW_Y + p.grid.offset(first) - p.scroll;

    for (r, row) in p.grid.rows.iter().enumerate().skip(first) {
        if y >= VIEW_Y + picker::VIEWPORT_H {
            break;
        }
        match row {
            Row::Header(title) => {
                set_source(cr, theme.window_fg.blend(theme.window_bg, HEADER_ALPHA));
                let layout = layout_for(cr, &fonts.ui_bold, title);
                let (_, th) = layout.pixel_size();
                cr.move_to(PAD + 4.0, y + grid::HEADER_H - th as f64 - 4.0);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Row::Cells(cells) => {
                for (c, ch) in cells.iter().enumerate() {
                    let x = PAD + c as f64 * grid::CELL;
                    if (r, c) == p.sel {
                        rounded_rect(cr, x, y, grid::CELL, grid::CELL, CELL_RADIUS);
                        set_source(cr, theme.selection_bg);
                        cr.fill().unwrap();
                    } else if p.hover == Some((r, c)) {
                        rounded_rect(cr, x, y, grid::CELL, grid::CELL, CELL_RADIUS);
                        set_source(cr, theme.window_fg.blend(theme.window_bg, HOVER_ALPHA));
                        cr.fill().unwrap();
                    }
                    let layout = layout_for(cr, &fonts.emoji, ch);
                    let (tw, th) = layout.pixel_size();
                    // Most cells are colour glyphs that ignore the source, but emoji with
                    // a text presentation are drawn in it, so the selected cell has to
                    // switch to the selection foreground to stay legible.
                    set_source(
                        cr,
                        if (r, c) == p.sel {
                            theme.selection_fg
                        } else {
                            theme.window_fg
                        },
                    );
                    cr.move_to(
                        x + (grid::CELL - tw as f64) / 2.0,
                        y + (grid::CELL - th as f64) / 2.0,
                    );
                    pangocairo::functions::show_layout(cr, &layout);
                }
            }
        }
        y += row.height();
    }
    cr.restore().unwrap();

    scrollbar(cr, theme, p);
}

/// A hairline scrollbar down the right edge of the viewport, drawn only when the list is
/// taller than the window.
fn scrollbar(cr: &Context, theme: &Theme, p: &Picker) {
    let total = p.grid.height();
    if total <= picker::VIEWPORT_H {
        return;
    }
    let track_h = picker::VIEWPORT_H;
    let thumb_h = (track_h * track_h / total).max(24.0);
    let travel = track_h - thumb_h;
    let progress = p.scroll / (total - picker::VIEWPORT_H);
    let x = CARD_W - PAD - 3.0;
    rounded_rect(
        cr,
        x,
        VIEW_Y + travel * progress.clamp(0.0, 1.0),
        3.0,
        thumb_h,
        1.5,
    );
    set_source(cr, theme.window_fg.blend(theme.window_bg, 0.25));
    cr.fill().unwrap();
}

fn footer(cr: &Context, theme: &Theme, fonts: &Fonts, p: &Picker) {
    let Some(text) = p
        .selected()
        .and_then(emoji::find)
        .map(|e| format!("{}  {}", e.ch, e.name))
    else {
        return;
    };
    set_source(cr, theme.window_fg.blend(theme.window_bg, FOOTER_ALPHA));
    let layout = layout_for(cr, &fonts.small, &text);
    let (_, th) = layout.pixel_size();
    cr.move_to(PAD + 4.0, FOOTER_Y + (FOOTER_H - th as f64) / 2.0);
    pangocairo::functions::show_layout(cr, &layout);
}

/// Top of a settings row, relative to the card.
fn setting_y(i: usize) -> f64 {
    TABS_Y + 8.0 + i as f64 * SETTING_ROW_H
}

/// Left edge of the tone swatches. They sit right-aligned in their row, where the other
/// rows put their value.
fn tone_strip_x() -> f64 {
    CONTROL_R - TONE_SWATCH * TONE_COUNT as f64
}

fn switch_x() -> f64 {
    CONTROL_R - SWITCH_W
}

fn stepper_x() -> f64 {
    CONTROL_R - STEP_W * 2.0 - STEP_VALUE_W
}

/// Which stepper button a card-relative point falls on: -1 for minus, 1 for plus.
pub fn stepper_at(x: f64, y: f64) -> Option<i32> {
    let i = picker::SETTINGS
        .iter()
        .position(|s| *s == Setting::RecentLimit)
        .expect("the recents row is in SETTINGS");
    let top = setting_y(i);
    if y < top || y >= top + SETTING_ROW_H - 4.0 {
        return None;
    }
    let x0 = stepper_x();
    if x >= x0 && x < x0 + STEP_W {
        return Some(-1);
    }
    let plus = x0 + STEP_W + STEP_VALUE_W;
    if x >= plus && x < plus + STEP_W {
        return Some(1);
    }
    None
}

/// Which category tab a card-relative point falls on. `count` is `Grid::sections.len()`,
/// since the bar always draws one slot per section.
pub fn tab_at(x: f64, y: f64, count: usize) -> Option<usize> {
    if count == 0 || y < TABS_Y || y >= TABS_Y + TABS_H {
        return None;
    }
    let slot = (CARD_W - PAD * 2.0) / count as f64;
    let rel = x - PAD;
    if rel < 0.0 {
        return None;
    }
    let i = (rel / slot) as usize;
    (i < count).then_some(i)
}

/// Index of the tone row in `SETTINGS`, so the backend can focus it on a swatch click.
pub fn tone_row() -> usize {
    picker::SETTINGS
        .iter()
        .position(|s| *s == Setting::Tone)
        .expect("the tone row is in SETTINGS")
}

/// Which tone swatch a card-relative point falls on, if any.
pub fn tone_at(x: f64, y: f64) -> Option<u8> {
    let top = setting_y(tone_row());
    if y < top || y >= top + SETTING_ROW_H - 4.0 {
        return None;
    }
    let rel = x - tone_strip_x();
    if rel < 0.0 {
        return None;
    }
    let i = (rel / TONE_SWATCH) as usize;
    (i < TONE_COUNT).then_some(i as u8)
}

/// Whether a card-relative point falls on the credit line's donate link.
pub fn donate_hit(x: f64, y: f64) -> bool {
    let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1) else {
        return false;
    };
    let Ok(cr) = cairo::Context::new(&surface) else {
        return false;
    };
    let fonts = Fonts::new();
    let (x0, head_w, link_w) = credit_metrics(&cr, &fonts);
    x >= x0 + head_w && x < x0 + head_w + link_w && y >= FOOTER_Y && y < FOOTER_Y + FOOTER_H
}

/// Whether a card-relative point falls in the search field.
pub fn search_hit(x: f64, y: f64) -> bool {
    x >= PAD && x < CARD_W - PAD - GEAR_W && y >= SEARCH_Y && y < SEARCH_Y + SEARCH_H
}

/// The byte offset in `text` that a card-relative x lands on, for click-to-position.
///
/// Shaping has to happen against a real context, so this measures on a throwaway 1x1
/// surface rather than trying to keep the frame's layout alive past the frame.
pub fn search_index_at(text: &str, x: f64) -> usize {
    let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1) else {
        return text.len();
    };
    let Ok(cr) = cairo::Context::new(&surface) else {
        return text.len();
    };
    let layout = layout_for(&cr, &Fonts::new().ui, text);
    let px = ((x - TEXT_X) * pango::SCALE as f64) as i32;
    let (inside, index, trailing) = layout.xy_to_index(px.max(0), 0);
    if !inside && px > 0 {
        // Past the last glyph: the caller means the end of the line.
        return text.len();
    }
    // `trailing` counts characters into the cluster the click fell on, so a click on the
    // right half of a character puts the cursor after it rather than before.
    let mut at = index as usize;
    for _ in 0..trailing {
        at = (at + 1..=text.len())
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(text.len());
    }
    at
}

/// Whether a card-relative point falls on the gear.
pub fn gear_hit(x: f64, y: f64) -> bool {
    x >= CARD_W - PAD - GEAR_W && x < CARD_W - PAD && y >= SEARCH_Y && y < SEARCH_Y + SEARCH_H
}

/// Which settings row a card-relative point falls on, if any.
pub fn setting_at(x: f64, y: f64) -> Option<usize> {
    if x < PAD || x >= CARD_W - PAD {
        return None;
    }
    let rel = y - setting_y(0);
    if rel < 0.0 {
        return None;
    }
    let i = (rel / SETTING_ROW_H) as usize;
    // The gap under each row is dead space rather than part of the next row.
    if rel - i as f64 * SETTING_ROW_H >= SETTING_ROW_H - 4.0 {
        return None;
    }
    (i < picker::SETTINGS.len()).then_some(i)
}

fn layout_for(cr: &Context, font: &FontDescription, text: &str) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(font));
    layout.set_text(text);
    layout
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
        assert_eq!(x, ((1920.0 - CARD_W) / 2.0).floor());
        assert_eq!(y, ((1200.0 - CARD_H) / 2.0).floor());
    }

    #[test]
    fn an_output_smaller_than_the_card_does_not_go_negative() {
        let (x, y) = card_origin(320.0, 200.0);
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn the_card_is_exactly_twelve_cells_wide_plus_padding() {
        assert_eq!(CARD_W, 12.0 * grid::CELL + 24.0);
        // And the regions stack up to its height without overlapping.
        assert_eq!(FOOTER_Y + FOOTER_H + PAD, CARD_H);
    }

    #[test]
    fn the_viewport_origin_sits_inside_the_card() {
        let (cx, cy) = card_origin(1920.0, 1200.0);
        let (vx, vy) = viewport_origin(1920.0, 1200.0);
        assert_eq!(vx, cx + PAD);
        assert_eq!(vy, cy + VIEW_Y);
        assert!(vy + picker::VIEWPORT_H < cy + CARD_H);
    }
}
