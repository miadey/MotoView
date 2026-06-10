//! Fluent 2 design-system theme for the native MotokoStudio shell.
//!
//! This module is the SINGLE SOURCE OF TRUTH for the studio's look. It mirrors
//! the Fluent UI 2 token foundation MotoView already uses on the web (brand =
//! MotoView purple `#6d28d9`) and bakes those tokens into an `egui::Style`:
//! window/panel/card fills, per-state widget visuals (fill + stroke +
//! corner-radius), the brand selection color, an 8px spacing grid, and a clear
//! typographic scale.
//!
//! It is deliberately PURE-ish: `apply_fluent_theme(ctx, dark)` is the only
//! side-effecting entry point (it installs the global style). Everything else
//! is plain color/`Style` construction so the palette can be reasoned about and
//! reused by the renderer (e.g. the Kanban board cards) without re-deriving
//! magic numbers.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals,
};

/// The full Fluent token palette for ONE theme (dark or light). Holding the
/// resolved colors in a struct keeps the panel/renderer code declarative — it
/// reads `palette.brand` / `palette.card` instead of hard-coding hexes.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// True for the dark theme — used by callers that need a light/dark branch
    /// (e.g. the code-editor syntax colors).
    pub dark: bool,

    // --- surfaces (back to front) ----------------------------------------
    /// App / window background — the deepest surface.
    pub window: Color32,
    /// Panel background (top bar, side panels, status bar).
    pub panel: Color32,
    /// Card / layer fill (deal cards, the editor card, column cards).
    pub card: Color32,
    /// Raised / hover fill — one step lighter than a card.
    pub raised: Color32,
    /// Hairline stroke / divider color.
    pub stroke: Color32,

    // --- text -------------------------------------------------------------
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,

    // --- brand accent -----------------------------------------------------
    pub brand: Color32,
    pub brand_hover: Color32,
    pub brand_pressed: Color32,
    /// Brand-subtle fill (selected rows / active chips) — translucent brand.
    pub brand_subtle: Color32,
    /// Text drawn ON the brand fill (primary buttons).
    pub on_brand: Color32,

    // --- semantic ---------------------------------------------------------
    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub info: Color32,
}

/// Solid #rrggbb.
const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    /// The DARK Fluent palette (the studio default).
    pub const fn dark() -> Self {
        Self {
            dark: true,
            window: rgb(0x1b, 0x1a, 0x1f),
            panel: rgb(0x1f, 0x1e, 0x24),
            card: rgb(0x26, 0x24, 0x2c),
            raised: rgb(0x2f, 0x2d, 0x37),
            stroke: rgb(0x3a, 0x37, 0x42),
            text_primary: rgb(0xf3, 0xf2, 0xf5),
            text_secondary: rgb(0xb9, 0xb6, 0xc2),
            text_disabled: rgb(0x6f, 0x6b, 0x78),
            brand: rgb(0x6d, 0x28, 0xd9),
            brand_hover: rgb(0x7c, 0x3a, 0xed),
            brand_pressed: rgb(0x5b, 0x21, 0xb6),
            // rgba(109,40,217,0.14) -> alpha 36/255.
            brand_subtle: Color32::from_rgba_premultiplied(0x18, 0x09, 0x30, 36),
            on_brand: rgb(0xff, 0xff, 0xff),
            error: rgb(0xe5, 0x48, 0x4d),
            warning: rgb(0xd9, 0x93, 0x0a),
            success: rgb(0x2f, 0xaa, 0x5f),
            info: rgb(0x6d, 0x28, 0xd9),
        }
    }

    /// The LIGHT Fluent palette.
    pub const fn light() -> Self {
        Self {
            dark: false,
            window: rgb(0xfa, 0xf9, 0xfc),
            panel: rgb(0xff, 0xff, 0xff),
            card: rgb(0xff, 0xff, 0xff),
            raised: rgb(0xf3, 0xf1, 0xf7),
            stroke: rgb(0xe6, 0xe3, 0xec),
            text_primary: rgb(0x1f, 0x1d, 0x24),
            text_secondary: rgb(0x56, 0x52, 0x5f),
            text_disabled: rgb(0x9b, 0x97, 0xa4),
            brand: rgb(0x6d, 0x28, 0xd9),
            brand_hover: rgb(0x7c, 0x3a, 0xed),
            brand_pressed: rgb(0x5b, 0x21, 0xb6),
            // rgba(109,40,217,0.14) over a white surface.
            brand_subtle: Color32::from_rgba_premultiplied(0xed, 0xe5, 0xfb, 255),
            on_brand: rgb(0xff, 0xff, 0xff),
            error: rgb(0xe5, 0x48, 0x4d),
            warning: rgb(0xb8, 0x86, 0x0b),
            success: rgb(0x2f, 0xaa, 0x5f),
            info: rgb(0x6d, 0x28, 0xd9),
        }
    }

    /// Pick by flag.
    pub const fn of(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

// --- shape / space tokens --------------------------------------------------

/// Corner radius for cards & buttons (6px).
pub const RADIUS_CARD: u8 = 6;
/// Corner radius for inputs (4px).
pub const RADIUS_INPUT: u8 = 4;
/// The base spacing unit of the 8px grid.
pub const SPACE: f32 = 8.0;

/// A 1px hairline stroke in the palette's divider color.
pub fn hairline(p: &Palette) -> Stroke {
    Stroke::new(1.0, p.stroke)
}

/// The soft elevation shadow used under preview cards. egui's shadow support is
/// limited, so this is a small, low-opacity drop used sparingly (column cards).
pub fn card_shadow(p: &Palette) -> Shadow {
    Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: if p.dark {
            Color32::from_black_alpha(90)
        } else {
            Color32::from_black_alpha(28)
        },
    }
}

/// Build the full Fluent `egui::Style` for the chosen theme and install it as
/// the global style. Call once on startup and again whenever the user toggles
/// dark/light.
pub fn apply_fluent_theme(ctx: &egui::Context, dark: bool) {
    let p = Palette::of(dark);
    let mut style = (*ctx.global_style()).clone();

    style.visuals = build_visuals(&p);
    apply_spacing(&mut style);
    apply_text_scale(&mut style);

    // Tighten interaction feel: a touch snappier hover/click animation.
    style.animation_time = 0.06;

    ctx.set_global_style(style);
}

/// The Fluent `Visuals` (colors) for a palette.
pub fn build_visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.dark_mode = p.dark;
    v.override_text_color = Some(p.text_primary);

    // Surfaces.
    v.window_fill = p.panel;
    v.panel_fill = p.panel;
    v.window_stroke = hairline(p);
    v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
    v.menu_corner_radius = CornerRadius::same(RADIUS_CARD);
    // `faint_bg_color` is used for striped rows / subtle fills; `extreme` for
    // text-edit backgrounds — make the editor surface a clean card.
    v.faint_bg_color = p.raised;
    v.extreme_bg_color = if p.dark { p.window } else { p.raised };

    // Semantic text colors.
    v.hyperlink_color = p.brand;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.warning;

    // Selection = brand-subtle fill + brand stroke (selectable rows, text
    // selection, active toggles).
    v.selection.bg_fill = p.brand_subtle;
    v.selection.stroke = Stroke::new(1.0, p.brand);

    // Soft elevation for free-floating windows/menus.
    v.window_shadow = card_shadow(p);
    v.popup_shadow = card_shadow(p);

    // Per-state widget visuals — the heart of the Fluent feel.
    let rc = CornerRadius::same(RADIUS_CARD);

    // Non-interactive (labels, separators, frame borders).
    v.widgets.noninteractive.bg_fill = p.panel;
    v.widgets.noninteractive.weak_bg_fill = p.panel;
    v.widgets.noninteractive.bg_stroke = hairline(p);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary);
    v.widgets.noninteractive.corner_radius = rc;

    // Inactive (resting buttons / inputs): card fill + hairline + primary text.
    v.widgets.inactive.bg_fill = p.card;
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_stroke = hairline(p);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);
    v.widgets.inactive.corner_radius = rc;
    v.widgets.inactive.expansion = 0.0;

    // Hovered: raised fill + a brand-tinted hairline.
    v.widgets.hovered.bg_fill = p.raised;
    v.widgets.hovered.weak_bg_fill = p.raised;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.brand);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_primary);
    v.widgets.hovered.corner_radius = rc;
    v.widgets.hovered.expansion = 1.0;

    // Active (pressed): brand-subtle fill + brand stroke.
    v.widgets.active.bg_fill = p.brand_subtle;
    v.widgets.active.weak_bg_fill = p.brand_subtle;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.brand);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text_primary);
    v.widgets.active.corner_radius = rc;
    v.widgets.active.expansion = 1.0;

    // Open (combo/menu open).
    v.widgets.open.bg_fill = p.raised;
    v.widgets.open.weak_bg_fill = p.raised;
    v.widgets.open.bg_stroke = hairline(p);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text_primary);
    v.widgets.open.corner_radius = rc;

    v
}

/// 8px spacing grid + comfortable button padding + panel margins.
fn apply_spacing(style: &mut egui::Style) {
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(SPACE, 6.0);
    s.button_padding = egui::vec2(12.0, 7.0);
    s.menu_margin = Margin::same(6);
    s.window_margin = Margin::same(12);
    s.indent = 18.0;
    s.interact_size.y = 28.0;
    s.icon_width = 18.0;
    s.icon_width_inner = 10.0;
    // A slightly slimmer scrollbar reads more "desktop app" than the default.
    s.scroll.bar_width = 10.0;
    s.scroll.floating = false;
}

/// The font size (px) used for section sub-headings. Applied directly via
/// `RichText::size` at call sites (rather than a registered `Name` text-style)
/// so it works on the very frame the theme is installed — `set_global_style`
/// only takes effect on the NEXT frame, so a freshly-built `Ui` would not yet
/// know a custom `Name` style.
pub const SUB_HEADING_SIZE: f32 = 15.0;

/// The typographic scale. Maps egui's built-in `TextStyle`s to a clear ramp.
/// All proportional except `Monospace` (the code editor / file:line refs).
fn apply_text_scale(style: &mut egui::Style) {
    use FontFamily::{Monospace, Proportional};
    let styles = &mut style.text_styles;
    styles.insert(TextStyle::Heading, FontId::new(20.0, Proportional));
    styles.insert(TextStyle::Body, FontId::new(14.0, Proportional));
    styles.insert(TextStyle::Button, FontId::new(14.0, Proportional));
    styles.insert(TextStyle::Small, FontId::new(12.0, Proportional));
    styles.insert(TextStyle::Monospace, FontId::new(13.0, Monospace));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_distinct_and_branded() {
        let d = Palette::dark();
        let l = Palette::light();
        // Brand is the MotoView purple in BOTH themes.
        assert_eq!(d.brand, rgb(0x6d, 0x28, 0xd9));
        assert_eq!(l.brand, rgb(0x6d, 0x28, 0xd9));
        // Dark vs light surfaces actually differ.
        assert_ne!(d.window, l.window);
        assert_ne!(d.text_primary, l.text_primary);
        assert!(d.dark && !l.dark);
    }

    #[test]
    fn visuals_apply_tokens() {
        let p = Palette::dark();
        let v = build_visuals(&p);
        assert_eq!(v.panel_fill, p.panel);
        assert_eq!(v.error_fg_color, p.error);
        assert_eq!(v.hyperlink_color, p.brand);
        assert_eq!(v.widgets.inactive.bg_fill, p.card);
        assert_eq!(v.widgets.inactive.corner_radius, CornerRadius::same(RADIUS_CARD));
    }
}
