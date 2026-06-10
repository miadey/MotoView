// Feasibility probe: can egui_kittest render an egui UI to a PNG with NO window
// (offscreen wgpu/Metal) in this environment? If yes, the studio redesign can be
// snapshot-rendered + visually reviewed headlessly.
//
// Run:  UPDATE_SNAPSHOTS=1 cargo test --manifest-path apps/studio/native/Cargo.toml --test snapshot_smoke
// It writes apps/studio/native/tests/snapshots/smoke.png

use egui_kittest::Harness;

#[test]
fn snapshot_smoke() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(720.0, 360.0))
        .build_ui(|ui| {
            ui.heading("MotokoStudio");
            ui.label("If this renders to a PNG, headless visual review works.");
            ui.horizontal(|ui| {
                let _ = ui.button("Open");
                let _ = ui.button("Check");
                let _ = ui.button("Preview");
            });
        });
    harness.run();
    harness.snapshot("smoke");
}
