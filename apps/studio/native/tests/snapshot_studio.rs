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
use motokostudio::app::{Device, StudioApp};

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
    // The studio now defaults to LIGHT, so set the theme EXPLICITLY for both
    // branches (a `true` here must actually flip the app to dark).
    app.set_dark(dark);
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

/// Copy a project tree (skipping build/noise dirs) so a drag-drop snapshot can
/// MUTATE the source without touching the real examples/crm.
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(name, ".mvbuild" | ".dfx" | ".git" | "target" | "node_modules") {
            continue;
        }
        let to = dst.join(name);
        if p.is_dir() {
            copy_tree(&p, &to);
        } else {
            std::fs::copy(&p, &to).unwrap();
        }
    }
}

/// An isolated copy of examples/crm (runtime package path absolutized) so a drop
/// can rewrite Board.mview without corrupting the committed project.
fn isolated_crm() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dst = std::env::temp_dir().join(format!("mvstudio_snap_crm_{}_{}", std::process::id(), nanos));
    copy_tree(&crm_dir(), &dst);
    let dfx = dst.join("dfx.json");
    if let Ok(txt) = std::fs::read_to_string(&dfx) {
        let repo = crm_dir().parent().unwrap().parent().unwrap().to_path_buf();
        let abs = repo.join("runtime/src");
        let _ = std::fs::write(&dfx, txt.replace("../../runtime/src", &abs.to_string_lossy()));
    }
    dst
}

/// DRAG-AND-DROP: drop a Button into the CRM header on an isolated copy and render
/// — the dropped component appears on the canvas (a Button below the Pipeline
/// title). Proves the toolbox tiles add real components.
#[test]
fn designer_dropped() {
    let dir = isolated_crm();
    let mut app = StudioApp::new_headless();
    app.open_project_path(&dir.display().to_string());
    app.open_first_file();
    app.run_preview();
    if app.board_summary().columns < 1 {
        eprintln!("SKIP: preview not runnable here (moc?)");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let ok = app.drop_component_into_class("crm-header", motokostudio::backend::ComponentKind::Button);
    assert!(ok, "should resolve + drop into the crm-header container");
    // After the drop the source was re-checked + re-previewed; the new Button is
    // now a real node in the forest.
    assert!(
        app.board_summary().columns >= 4,
        "the board should still render after the drop"
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1480.0, 920.0))
        .build_ui(move |ui| app.draw(ui));
    harness.run();
    harness.snapshot("designer_dropped");
    let _ = std::fs::remove_dir_all(&dir);
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

/// The GUI SELECTION: select a node and render the full studio — the selected
/// node's source location appears in the preview header and the node(s) get a
/// brand outline. Selecting the `deal-card` template highlights every instance.
#[test]
fn studio_crm_selected() {
    let mut app = studio_with_crm(true);
    assert_board_loaded(&app);
    assert!(
        app.select_node_by_class("deal-card"),
        "a source-mapped deal-card should be selectable (forest must carry data-mv-src)"
    );
    let loc = app
        .selected_source_location()
        .expect("the selected node resolves to a source location");
    assert!(loc.contains("Board.mview"), "source location = {loc}");
    assert!(loc.contains("<article>"), "source location = {loc}");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui(move |ui| app.draw(ui));
    harness.run();
    harness.snapshot("studio_crm_selected");
}

// ---------------------------------------------------------------------------
// VISUAL DESIGNER snapshots — the Canva-style layout: a friendly element toolbox
// (left), the design canvas with a soft gray work-surface + a floating white
// device frame (center), and the inspector (right). Rendered LARGE and LIGHT —
// light is the studio's DEFAULT now (Canva is bright), so these prove the
// resting look. A single `designer_desktop_dark` preserves a dark sample.
// ---------------------------------------------------------------------------

/// Render the full designer at a designer-review size.
fn designer_snapshot(app: StudioApp, name: &str) {
    let mut app = app;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1480.0, 920.0))
        .build_ui(move |ui| app.draw(ui));
    harness.run();
    harness.snapshot(name);
}

/// DESKTOP frame (the default), LIGHT: the widest device, a plain window chrome,
/// a white frame floating on the gray Canva work surface.
#[test]
fn designer_desktop() {
    let mut app = studio_with_crm(false);
    assert_board_loaded(&app);
    app.set_device(Device::Desktop);
    assert_eq!(app.device(), Device::Desktop);
    designer_snapshot(app, "designer_desktop");
}

/// DESKTOP frame in DARK — a preserved dark sample so the dark theme keeps a
/// designer-level snapshot even though LIGHT is now the default.
#[test]
fn designer_desktop_dark() {
    let mut app = studio_with_crm(true);
    assert_board_loaded(&app);
    app.set_device(Device::Desktop);
    assert_eq!(app.device(), Device::Desktop);
    designer_snapshot(app, "designer_desktop_dark");
}

/// WEB frame, LIGHT: a faux browser (3 dots + an address pill) at ~1024px — the
/// device switch visibly narrows the frame + changes the chrome.
#[test]
fn designer_web() {
    let mut app = studio_with_crm(false);
    assert_board_loaded(&app);
    app.set_device(Device::Web);
    assert_eq!(app.device(), Device::Web);
    designer_snapshot(app, "designer_web");
}

/// MOBILE frame, LIGHT: a phone bezel at ~390px — the narrowest device, so the
/// board columns reflow inside the bezel (a genuinely responsive preview).
#[test]
fn designer_mobile() {
    let mut app = studio_with_crm(false);
    assert_board_loaded(&app);
    app.set_device(Device::Mobile);
    assert_eq!(app.device(), Device::Mobile);
    designer_snapshot(app, "designer_mobile");
}

/// INSPECTOR populated, LIGHT: select a node → the right inspector shows its tag,
/// source location, attributes + events, and the node is outlined on the canvas.
#[test]
fn designer_inspector() {
    let mut app = studio_with_crm(false);
    assert_board_loaded(&app);
    assert!(
        app.select_node_by_class("deal-card"),
        "a source-mapped deal-card should be selectable (forest must carry data-mv-src)"
    );
    let loc = app
        .selected_source_location()
        .expect("the selected node resolves to a source location");
    assert!(loc.contains("Board.mview"), "source location = {loc}");
    designer_snapshot(app, "designer_inspector");
}
