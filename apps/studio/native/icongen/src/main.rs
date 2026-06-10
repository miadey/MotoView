//! icongen — pure-Rust macOS iconset generator for MotokoStudio.
//!
//! Reads an SVG (the MotoView brand mark) and renders the full set of PNGs that
//! `iconutil -c icns` expects inside an `*.iconset/` directory:
//!
//!   icon_16x16.png       (16)    icon_16x16@2x.png    (32)
//!   icon_32x32.png       (32)    icon_32x32@2x.png    (64)
//!   icon_128x128.png     (128)   icon_128x128@2x.png  (256)
//!   icon_256x256.png     (256)   icon_256x256@2x.png  (512)
//!   icon_512x512.png     (512)   icon_512x512@2x.png  (1024)
//!
//! Each PNG is rendered DIRECTLY at its target pixel size from the vector source
//! (not downscaled from one big raster), so every size is crisp — the @2x of a
//! size is just the next size's pixel count, which we render natively too.
//!
//! NO system SVG renderer is used (rsvg/resvg/inkscape are absent on this box).
//! Rasterization is 100% Rust via usvg + resvg + tiny-skia.
//!
//! Usage:
//!   icongen <input.svg> <output.iconset-dir>
//!
//! It creates <output.iconset-dir> (and parents) and writes the 10 PNGs.

use std::path::Path;
use std::process::ExitCode;

/// The (filename, pixel-size) pairs macOS iconsets require. The @2x entries are
/// the same image at double the pixel count.
const ENTRIES: &[(&str, u32)] = &[
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: icongen <input.svg> <output.iconset-dir>");
        return ExitCode::from(2);
    }
    let svg_path = Path::new(&args[1]);
    let out_dir = Path::new(&args[2]);

    match run(svg_path, out_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("icongen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(svg_path: &Path, out_dir: &Path) -> Result<(), String> {
    let svg_data = std::fs::read(svg_path)
        .map_err(|e| format!("reading {}: {e}", svg_path.display()))?;

    // Parse the SVG into a usvg tree once; we re-rasterize it at each size.
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_data, &opt)
        .map_err(|e| format!("parsing {}: {e}", svg_path.display()))?;

    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("creating {}: {e}", out_dir.display()))?;

    let svg_size = tree.size();

    for (name, px) in ENTRIES {
        let px = *px;
        let mut pixmap = tiny_skia::Pixmap::new(px, px)
            .ok_or_else(|| format!("allocating {px}x{px} pixmap"))?;

        // Uniform scale that maps the SVG's intrinsic size onto a px*px canvas.
        // Our source is a square 1024x1024 viewBox, so width == height; using
        // the width scale keeps it exact and would letterbox cleanly if not.
        let scale = px as f32 / svg_size.width();
        let transform = tiny_skia::Transform::from_scale(scale, scale);

        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let png = pixmap
            .encode_png()
            .map_err(|e| format!("encoding {name}: {e}"))?;
        let dest = out_dir.join(name);
        std::fs::write(&dest, png)
            .map_err(|e| format!("writing {}: {e}", dest.display()))?;
        println!("  wrote {} ({px}x{px})", dest.display());
    }

    Ok(())
}
