//! Visual snapshot tests for the redesigned Fluent studio shell.
//!
//! These render the FULL studio (top bar + file panel + editor + diagnostics +
//! the Kanban preview) and a FOCUSED Kanban board to PNGs via egui_kittest's
//! offscreen wgpu/Metal renderer — no window is opened. They are the artifact a
//! human reviews for taste.
//!
//! Run (writes/updates the PNGs):
//!   UPDATE_SNAPSHOTS=1 cargo test --manifest-path apps/studio/native/Cargo.toml \
//!       --test snapshot_studio
//!
//! Each test loads the real `examples/crm` project (open_project + run_preview,
//! which spawns the committed `motoview` release binary) so the board shows the
//! genuine Pipeline / Lead / Contacted / Proposal / Won columns with real deal
//! cards — NOT a mock.

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use motokostudio::app::StudioApp;

/// Absolute path to the repo's `examples/crm` project (crate is at
/// `<repo>/apps/studio/native`, so climb three levels to the repo root).
fn crm_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root above apps/studio/native")
        .join("examples/crm")
}

/// Build a studio app with the CRM project loaded, a file open in the editor,
/// and a fresh preview rendered. `dark` selects the Fluent theme.
fn studio_with_crm(dark: bool) -> StudioApp {
    let mut app = StudioApp::new_headless();
    if !dark {
        app.set_dark(false);
    }
    let dir = crm_dir();
    assert!(
        dir.join("src/Pages/Board.mview").exists(),
        "CRM project must exist at {} (with src/Pages/Board.mview)",
        dir.display()
    );
    app.open_project_path(&dir.display().to_string());
    app.open_first_file();
    app.run_preview();
    app
}

/// Assert the preview actually rendered a non-empty CRM board (a real Kanban),
/// so a green snapshot can never silently hide an empty board.
fn assert_board_loaded(app: &StudioApp) {
    let board = app.board_summary();
    assert!(
        board.columns >= 4,
        "expected >=4 kanban columns, got {} (preview failed to load the CRM board)",
        board.columns
    );
    assert!(
        board.deal_cards >= 1,
        "expected >=1 deal card, got {}",
        board.deal_cards
    );
    // Every interactive board button must carry a readable, non-empty label —
    // this is the empty-square regression guard.
    assert!(
        board.empty_button_labels == 0,
        "{} board button(s) would render as an empty square",
        board.empty_button_labels
    );
}

#[test]
fn studio_crm_dark() {
    let mut app = studio_with_crm(true);
    assert_board_loaded(&app);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui(move |ui| app.draw(ui));
    harness.run();
    harness.snapshot("studio_crm_dark");
}

#[test]
fn studio_crm_light() {
    let mut app = studio_with_crm(false);
    assert_board_loaded(&app);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui(move |ui| app.draw(ui));
    harness.run();
    harness.snapshot("studio_crm_light");
}

#[test]
fn preview_kanban_dark() {
    let mut app = studio_with_crm(true);
    assert_board_loaded(&app);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 780.0))
        .build_ui(move |ui| app.draw_preview_only(ui));
    harness.run();
    harness.snapshot("preview_kanban_dark");
}
