//! Brand-ramp theming for `@theme brand="#hex"`.
//!
//! Given a brand color, generate the 16-step Fluent brand ramp (10..160) and
//! emit the brand alias tokens for light AND dark, exactly the way Fluent's
//! createLightTheme/createDarkTheme(brandRamp) wire them. The ramp anchors shade
//! 80 to the input color and follows the reference ramp's lightness ladder, so
//! any brand color yields a cohesive, Fluent-shaped ramp.

/// The reference brand ramp (Communication blue #0f6cbd), shades 10..160.
const REF_RAMP: [&str; 16] = [
    "#061724", "#082338", "#0a2e4a", "#0c3b5e", "#0e4775", "#0f548c", "#115ea3", "#0f6cbd",
    "#2886de", "#479ef5", "#62abf5", "#77b7f7", "#96c6fa", "#b4d6fa", "#cfe4fa", "#ebf3fc",
];

include!("brand_aliases.rs");

fn parse_hex(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().trim_start_matches('#');
    let h = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_string(),
        _ => return None,
    };
    let r = u8::from_str_radix(&h[0..2], 16).ok()? as f64 / 255.0;
    let g = u8::from_str_radix(&h[2..4], 16).ok()? as f64 / 255.0;
    let b = u8::from_str_radix(&h[4..6], 16).ok()? as f64 / 255.0;
    Some((r, g, b))
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - r).abs() < 1e-9 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-9 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 { t += 1.0 }
    if t > 1.0 { t -= 1.0 }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 1.0 / 2.0 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    p
}

fn hsl_to_hex(h: f64, s: f64, l: f64) -> String {
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    let (r, g, b) = if s.abs() < 1e-9 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (hue_to_rgb(p, q, h + 1.0 / 3.0), hue_to_rgb(p, q, h), hue_to_rgb(p, q, h - 1.0 / 3.0))
    };
    let to = |v: f64| ((v * 255.0).round() as i64).clamp(0, 255);
    format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b))
}

fn hex_to_hsl(hex: &str) -> Option<(f64, f64, f64)> {
    parse_hex(hex).map(|(r, g, b)| rgb_to_hsl(r, g, b))
}

/// The 16-step ramp for a brand color: shade 80 == the input, other shades
/// follow the reference ramp's lightness deltas with brand-matched saturation.
fn brand_ramp(hex: &str) -> Option<[String; 16]> {
    let (bh, bs, bl) = hex_to_hsl(hex)?;
    let (_, ref_s80, ref_l80) = hex_to_hsl(REF_RAMP[7]).unwrap();
    let sat_scale = if ref_s80 > 1e-3 { bs / ref_s80 } else { 1.0 };
    let mut out: [String; 16] = Default::default();
    for i in 0..16 {
        let (_, ref_s, ref_l) = hex_to_hsl(REF_RAMP[i]).unwrap();
        let l = (bl + (ref_l - ref_l80)).clamp(0.02, 0.985);
        let s = (ref_s * sat_scale).clamp(0.0, 1.0);
        out[i] = hsl_to_hex(bh, s, l);
    }
    Some(out)
}

fn shade_idx(shade: u32) -> usize {
    (shade / 10).saturating_sub(1) as usize
}

/// The `<style>:root{…}</style>` for `@theme brand="#hex"`: the 16-shade ramp +
/// the light brand aliases in :root, and the dark brand aliases under
/// [data-theme="dark"] and the prefers-color-scheme media query.
pub fn brand_theme_css(hex: &str) -> Option<String> {
    let ramp = brand_ramp(hex)?;
    let mut root = String::new();
    for i in 0..16 {
        root.push_str(&format!("--colorBrandBackground{}:{};", (i + 1) * 10, ramp[i]));
    }
    for (tok, light, _dark) in BRAND_ALIASES {
        root.push_str(&format!("--{}:{};", tok, ramp[shade_idx(*light)]));
    }
    let mut dark = String::new();
    for (tok, _light, d) in BRAND_ALIASES {
        dark.push_str(&format!("--{}:{};", tok, ramp[shade_idx(*d)]));
    }
    Some(format!(
        "<style>:root{{{root}}}[data-theme=\"dark\"]{{{dark}}}@media (prefers-color-scheme: dark){{:root:not([data-theme=\"light\"]){{{dark}}}}}</style>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ramp_anchors_shade_80_to_input() {
        let r = brand_ramp("#0f6cbd").unwrap();
        // shade 80 should reproduce the input (within rounding)
        assert_eq!(r[7], "#0f6cbd", "shade 80 must equal the brand input");
    }
    #[test]
    fn brand_css_sets_background_and_dark() {
        let css = brand_theme_css("#d13438").unwrap(); // a red brand
        assert!(css.contains("--colorBrandBackground:"), "brand background emitted");
        assert!(css.contains("[data-theme=\"dark\"]"), "dark overrides emitted");
        assert!(css.contains("--colorBrandBackground10:"), "ramp emitted");
    }
}
