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
mod app;

use app::StudioApp;

fn main() -> eframe::Result<()> {
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
