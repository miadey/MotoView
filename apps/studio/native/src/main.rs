//! MotokoStudio (native) — entry point.
//!
//! A genuinely webview-free desktop IDE for MotoView: eframe (egui + a window)
//! on the `wgpu` backend, which renders through Metal on macOS. The only system
//! dependencies are the linker and the GPU driver — on macOS that means AppKit /
//! Metal / QuartzCore from the Command-Line-Tools SDK (Metal.framework lives in
//! /System/Library/Frameworks and is linkable via the CLT linker). We picked
//! wgpu/Metal over the legacy glow/OpenGL backend because Apple has deprecated
//! OpenGL — Metal is the future-proof path and is still a SYSTEM framework, so
//! there is NO WebKit, NO embedded browser, NO Xcode requirement, NO
//! code-signing, NO $99.
//!
//! Run with:  cargo run --manifest-path apps/studio/native/Cargo.toml
//! (needs a desktop GUI session — it opens a real window.)

// The bin re-uses the library crate's tested modules (via `app`, which imports
// from the `motokostudio` library) rather than recompiling the sources
// standalone — that avoids dead-code noise for backend functions the GUI
// doesn't call but the tests do, and guarantees the binary links the exact code
// that is unit-tested.
use std::io::Write;
use std::path::PathBuf;

use motokostudio::app::StudioApp;

/// Where the panic hook appends crash reports. Prefers the standard macOS
/// per-user log dir (`~/Library/Logs/MotokoStudio/crash.log`), falling back to
/// the OS temp dir if `$HOME` is somehow unset. Returned so `main` can print
/// the location once at startup (so the user knows where to look).
fn crash_log_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let dir = PathBuf::from(home).join("Library/Logs/MotokoStudio");
        // Best-effort; if this fails we still fall back below at write time.
        let _ = std::fs::create_dir_all(&dir);
        return dir.join("crash.log");
    }
    std::env::temp_dir().join("MotokoStudio-crash.log")
}

/// Install a panic hook so a crash is DIAGNOSABLE instead of a silent exit.
/// The original problem ("the app crashes + exits when opening a folder") was
/// invisible because nothing logged the panic. This writes the message +
/// location to stderr AND appends it (with a timestamp-ish marker) to the
/// crash log file. We keep eframe's own run flow intact.
fn install_panic_hook(log_path: PathBuf) {
    // Chain onto the default hook so we keep the normal backtrace behaviour.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let report = format!(
            "[MotokoStudio PANIC] at {location}: {msg}\n  (pid {})\n",
            std::process::id()
        );

        // Always to stderr.
        eprint!("{report}");

        // And append to the log file (best-effort; ignore IO errors so the hook
        // itself can never panic). Fall back to temp dir if the chosen path is
        // unwritable.
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .and_then(|mut f| f.write_all(report.as_bytes()));
        if appended.is_err() {
            let fallback = std::env::temp_dir().join("MotokoStudio-crash.log");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&fallback)
            {
                let _ = f.write_all(report.as_bytes());
            }
        }

        // Preserve default behaviour (backtrace etc.).
        default(info);
    }));
}

fn main() -> eframe::Result<()> {
    let log_path = crash_log_path();
    eprintln!(
        "MotokoStudio: crash reports (if any) will be written to {}",
        log_path.display()
    );
    install_panic_hook(log_path);

    let options = eframe::NativeOptions {
        // Future-proof renderer: wgpu -> Metal on macOS (OpenGL/glow is
        // deprecated by Apple). Metal is a system framework via the CLT linker,
        // so this keeps the zero-Xcode / zero-webview / zero-signing property.
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MotokoStudio — native (webview-free)")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MotokoStudio",
        options,
        Box::new(|cc| Ok(Box::new(StudioApp::new(cc)))),
    )
}
