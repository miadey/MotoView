//! The egui/eframe GUI shell — a Fluent 2 desktop IDE for MotoView.
//!
//! Deliberately THIN: every decision of substance (which widget renders a node,
//! how to parse diagnostics, file IO, spawning the compiler) lives in `backend`
//! / `highlight` and is unit-tested headless. The LOOK comes from
//! [`crate::theme`]: a Fluent token foundation (brand = MotoView `#6d28d9`).
//!
//! Layout:
//!   * top    : wordmark + brand dot, project-path input + Open, an action
//!              cluster (Check / Lint / Preview / Save), the route field, and a
//!              dark/light toggle.
//!   * left   : a titled file panel — each `.mview` a selectable ROW.
//!   * center : the code editor in a card (empty-state when nothing is open) +
//!              the diagnostics list with severity chips.
//!   * right  : the PREVIEW panel — a real Fluent KANBAN board built from the
//!              page IR forest (the 4th renderer of the same IR).
//!
//! Drawing is driven from `draw(&mut self, ui)` so the SAME code path renders
//! both in the live eframe window (`App::ui`) and in the headless egui_kittest
//! snapshot harness.

use std::path::PathBuf;

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Response, RichText, Stroke,
    TextStyle, Ui,
};

use crate::backend::{
    self, CommandReport, OpenOutcome, PreviewResult, Session, SessionEvent, UiNode, WidgetKind,
};
use crate::highlight::{classify_line, TokenClass};
use crate::theme::{self, Palette, RADIUS_CARD, RADIUS_INPUT, SPACE};

/// A testable summary of the previewed CRM Kanban board (see
/// [`StudioApp::board_summary`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoardSummary {
    /// Number of `kanban-col` columns found in the forest.
    pub columns: usize,
    /// Number of `deal-card` cards found.
    pub deal_cards: usize,
    /// Number of per-card action buttons wired (remove / back / fwd).
    pub action_buttons: usize,
    /// Number of interactive board buttons that would render with an EMPTY
    /// label — must be 0.
    pub empty_button_labels: usize,
}

/// The whole application state.
pub struct StudioApp {
    project_dir: Option<PathBuf>,
    /// The editable project-folder path shown in the top bar. The user pastes
    /// or types a path here and clicks "Open" — this is the PRIMARY, always-
    /// works way to open a project. It never triggers a native folder dialog
    /// (which crashes the macOS run-loop when run synchronously from update()).
    project_path_input: String,
    files: Vec<PathBuf>,
    open_file: Option<PathBuf>,
    editor_text: String,
    dirty: bool,

    diagnostics: Option<CommandReport>,
    preview: Option<PreviewResult>,
    route: String,
    status: String,

    /// Dark (true) vs light (false) Fluent theme. Toggled from the top bar; the
    /// global style is re-applied whenever it flips.
    dark: bool,
    /// Set on the frame the theme flips so `draw` re-installs the global style.
    theme_dirty: bool,

    // --- R17: LIVE preview via replay -------------------------------------
    /// The accumulated, ordered events dispatched in the current preview
    /// session. Replaying the whole list reproduces page-local state from the
    /// initial render up to the latest click.
    session: Session,
    /// The route the CURRENT preview/session was rendered for, captured at
    /// Preview time so replays stay pinned to the same page even if the user
    /// edits the route box afterwards.
    session_route: Option<String>,
    /// A last replay error to surface in the diagnostics area (cleared on the
    /// next successful dispatch / preview / reset).
    replay_error: Option<String>,
}

impl Default for StudioApp {
    fn default() -> Self {
        Self {
            project_dir: None,
            project_path_input: String::new(),
            files: Vec::new(),
            open_file: None,
            editor_text: String::new(),
            dirty: false,
            diagnostics: None,
            preview: None,
            route: String::new(),
            status: "Open a MotoView project (a folder with motoview.json) to begin.".to_string(),
            dark: true,
            theme_dirty: true,
            session: Vec::new(),
            session_route: None,
            replay_error: None,
        }
    }
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Install the Fluent dark theme immediately so the very first frame is
        // already on-brand (no flash of default egui chrome).
        theme::apply_fluent_theme(&cc.egui_ctx, true);

        let mut app = Self::default();
        app.theme_dirty = false; // already applied above

        // Pre-fill the path field with the current working dir so the user has a
        // sensible starting point and the Open button is one click away.
        if let Ok(cwd) = std::env::current_dir() {
            app.project_path_input = cwd.display().to_string();
        }

        // CLI arg wins: `cargo run -- <dir>` (or the bundled binary + a path)
        // opens that project immediately — fully non-interactive, no dialog.
        let cli_dir = std::env::args().skip(1).find(|a| !a.starts_with('-'));
        if let Some(arg) = cli_dir {
            app.project_path_input = arg.clone();
            app.open_project_from_input();
        } else if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("motoview.json").exists() {
                app.open_project_from_input();
            }
        }
        app
    }

    /// A bare app for the snapshot harness (no `CreationContext`). The first
    /// `draw`/`draw_preview_only` call installs the Fluent theme on the
    /// harness's `Context` (because `theme_dirty` defaults to true).
    pub fn new_headless() -> Self {
        Self::default()
    }

    /// Select the Fluent theme (true = dark). Re-applied on the next draw.
    pub fn set_dark(&mut self, dark: bool) {
        if dark != self.dark {
            self.dark = dark;
            self.theme_dirty = true;
        }
    }

    /// A small, testable summary of the currently-previewed CRM board: column
    /// count, deal-card count, and how many interactive board buttons WOULD
    /// render with an empty label (the empty-square regression guard). Computed
    /// from the live preview forest using the exact label logic the renderer
    /// uses, so a passing count guarantees the snapshot has no blank buttons.
    pub fn board_summary(&self) -> BoardSummary {
        let mut s = BoardSummary::default();
        let Some(pr) = self.preview.as_ref().filter(|p| p.ok) else {
            return s;
        };
        let mut columns = Vec::new();
        let mut cards = Vec::new();
        for node in &pr.forest {
            find_all_class(node, "kanban-col", &mut columns);
            find_all_class(node, "deal-card", &mut cards);
        }
        s.columns = columns.len();
        s.deal_cards = cards.len();

        // Header "+ New deal" button.
        for node in &pr.forest {
            if let Some(btn) = find_button_with_handler(node, "toggleCreate") {
                if nonempty_label(&backend::node_text(btn), "+ New deal").is_empty() {
                    s.empty_button_labels += 1;
                }
            }
        }
        // Per-card action buttons all render with FIXED readable labels
        // ("Del" / "Back" / "Fwd"), so they can never be empty — count them as
        // present to prove the wiring (a card without its buttons is a bug).
        for card in &cards {
            for (handler, label) in [
                ("removeDeal", "Del"),
                ("moveBack", "Back"),
                ("moveFwd", "Fwd"),
            ] {
                if find_button_with_handler(card, handler).is_some() {
                    s.action_buttons += 1;
                    debug_assert!(!label.is_empty());
                }
            }
        }
        s
    }

    /// Open the project named by the current `project_path_input`. Crash-free:
    /// resolves the typed/pasted path via the pure, tested `backend::open_project`
    /// (NO native NSOpenPanel) and either loads the project or sets a status.
    fn open_project_from_input(&mut self) {
        let input = self.project_path_input.clone();
        match backend::open_project(&input) {
            OpenOutcome::Opened { dir, files, note } => {
                self.project_path_input = dir.display().to_string();
                self.apply_project(dir, files, note);
            }
            OpenOutcome::Rejected { note } => {
                self.status = note;
            }
        }
    }

    /// Adopt a resolved project (directory + its `.mview` files) into the app
    /// state. Pure state mutation — no filesystem access, no dialogs.
    pub fn apply_project(&mut self, dir: PathBuf, files: Vec<PathBuf>, note: String) {
        self.files = files;
        self.status = note;
        self.project_dir = Some(dir);
        self.open_file = None;
        self.editor_text.clear();
        self.dirty = false;
        self.diagnostics = None;
        self.preview = None;
        self.session.clear();
        self.session_route = None;
        self.replay_error = None;
    }

    /// Open the project at `path` (used by the snapshot harness directly).
    pub fn open_project_path(&mut self, path: &str) {
        self.project_path_input = path.to_string();
        self.open_project_from_input();
    }

    fn open(&mut self, path: PathBuf) {
        match backend::read_file(&path) {
            Ok(text) => {
                self.editor_text = text;
                self.dirty = false;
                self.status = format!("Opened {}", path.display());
                self.open_file = Some(path);
            }
            Err(e) => self.status = format!("Failed to read file: {e}"),
        }
    }

    /// Open the first `.mview` file (used by the snapshot harness to populate
    /// the editor pane).
    pub fn open_first_file(&mut self) {
        if let Some(f) = self.files.first().cloned() {
            self.open(f);
        }
    }

    fn save(&mut self) {
        if let Some(path) = self.open_file.clone() {
            match backend::write_file(&path, &self.editor_text) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = format!("Saved {}", path.display());
                }
                Err(e) => self.status = format!("Save failed: {e}"),
            }
        }
    }

    fn run_check(&mut self) {
        if let Some(dir) = &self.project_dir {
            let r = backend::run_check(dir);
            self.status = summarize(&r, "check");
            self.diagnostics = Some(r);
        }
    }

    fn run_lint(&mut self) {
        if let Some(dir) = &self.project_dir {
            let r = backend::run_lint(dir);
            self.status = summarize(&r, "lint");
            self.diagnostics = Some(r);
        }
    }

    pub fn run_preview(&mut self) {
        if let Some(dir) = &self.project_dir {
            let route = if self.route.trim().is_empty() {
                None
            } else {
                Some(self.route.trim())
            };
            let r = backend::run_preview(dir, route);
            self.status = if r.ok {
                format!("Preview: {} root node(s).", r.forest.len())
            } else {
                format!("Preview failed: {}", r.note.clone().unwrap_or_default())
            };
            self.preview = Some(r);
            self.session.clear();
            self.session_route = route.map(str::to_string);
            self.replay_error = None;
        }
    }

    // --- R17: LIVE preview via replay -------------------------------------

    fn dispatch_event(&mut self, ev: SessionEvent) {
        let Some(dir) = self.project_dir.clone() else {
            return;
        };
        self.session.push(ev);
        let route = self.session_route.as_deref();
        match backend::replay_dispatch(&dir, route, &self.session) {
            Ok(forest) => {
                let n = forest.len();
                self.preview = Some(PreviewResult {
                    forest,
                    raw_json: String::new(),
                    note: None,
                    ok: true,
                });
                self.replay_error = None;
                self.status = format!(
                    "Live: dispatched {} event(s) — {} root node(s).",
                    self.session.len(),
                    n
                );
            }
            Err(e) => {
                self.session.pop();
                self.replay_error = Some(e.clone());
                self.status = format!("Replay failed: {e}");
            }
        }
    }

    fn reset_session(&mut self) {
        self.session.clear();
        self.replay_error = None;
        if self.project_dir.is_some() {
            self.run_preview();
            self.status = "Live session reset to initial render.".to_string();
        }
    }
}

fn summarize(r: &CommandReport, what: &str) -> String {
    let errs = r.diagnostics.iter().filter(|d| d.is_error()).count();
    let warns = r.diagnostics.iter().filter(|d| d.is_warning()).count();
    let mut s = format!("{what}: {errs} error(s), {warns} warning(s)");
    if let Some(note) = &r.note {
        s.push_str(&format!(" — {note}"));
    }
    s
}

impl eframe::App for StudioApp {
    // eframe 0.34 calls `ui(ui, frame)` with a root `Ui` (the legacy
    // `update(ctx, frame)` is deprecated). We draw everything through `draw`,
    // which uses `Panel::*::show_inside(ui, …)` — zero deprecated APIs.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl StudioApp {
    /// Render the whole studio into a root `Ui`. Single entry point shared by
    /// the live window and the headless snapshot harness.
    pub fn draw(&mut self, ui: &mut Ui) {
        // Re-apply the global theme if the dark/light toggle flipped this frame.
        if self.theme_dirty {
            theme::apply_fluent_theme(ui.ctx(), self.dark);
            self.theme_dirty = false;
        }
        let p = Palette::of(self.dark);

        self.top_bar(ui, &p);
        self.bottom_bar(ui, &p);
        self.left_panel(ui, &p);
        self.right_panel(ui, &p);
        self.central_editor(ui, &p);
    }

    /// Draw ONLY the Kanban board for the current preview forest, filling `ui`.
    /// Used by the focused `preview_kanban_*` snapshot. Applies the theme so the
    /// board renders on the branded surface (no surrounding chrome). Click
    /// dispatch is disabled here (snapshots are static).
    pub fn draw_preview_only(&mut self, ui: &mut Ui) {
        if self.theme_dirty {
            theme::apply_fluent_theme(ui.ctx(), self.dark);
            self.theme_dirty = false;
        }
        let p = Palette::of(self.dark);
        let frame = Frame::new()
            .fill(p.window)
            .inner_margin(Margin::same(16));
        frame.show(ui, |ui| {
            section_title(ui, &p, "Preview · Kanban board");
            ui.add_space(SPACE);
            let mut sink: Option<SessionEvent> = None;
            match &self.preview {
                Some(pr) if pr.ok => render_forest(ui, &p, &pr.forest, &mut sink),
                _ => empty_state(ui, &p, "▦", "No preview", "Load a project and run preview."),
            }
        });
    }

    fn top_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::symmetric(14, 10))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::top("top")
            .frame(frame)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    // Wordmark + brand dot.
                    brand_dot(ui, p);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("MotokoStudio")
                            .text_style(TextStyle::Heading)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.label(
                        RichText::new("native · webview-free")
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );

                    ui.add_space(SPACE);
                    vsep(ui, p);
                    ui.add_space(SPACE);

                    // Project path input + Open.
                    let path_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.project_path_input)
                            .hint_text("/path/to/project")
                            .desired_width(300.0),
                    );
                    let open_clicked = secondary_button(ui, p, "Open").clicked();
                    let enter_pressed = path_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if open_clicked || enter_pressed {
                        self.open_project_from_input();
                    }

                    ui.add_space(SPACE);
                    vsep(ui, p);
                    ui.add_space(SPACE);

                    // Action cluster (Check / Lint / route / Preview / Save).
                    let has_project = self.project_dir.is_some();
                    let mut do_check = false;
                    let mut do_lint = false;
                    let mut do_preview = false;
                    ui.add_enabled_ui(has_project, |ui| {
                        do_check = secondary_button(ui, p, "Check").clicked();
                        do_lint = secondary_button(ui, p, "Lint").clicked();
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("route")
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.route)
                                .hint_text("/")
                                .desired_width(96.0),
                        );
                        // Preview is the PRIMARY action — brand-filled.
                        do_preview = primary_button(ui, p, "Preview").clicked();
                    });
                    if do_check {
                        self.run_check();
                    }
                    if do_lint {
                        self.run_lint();
                    }
                    if do_preview {
                        self.run_preview();
                    }

                    let dirty = self.dirty;
                    let mut do_save = false;
                    ui.add_enabled_ui(self.open_file.is_some(), |ui| {
                        let label = if dirty { "Save *" } else { "Save" };
                        do_save = secondary_button(ui, p, label).clicked();
                    });
                    if do_save {
                        self.save();
                    }

                    // Theme toggle pushed to the far right.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let icon = if self.dark { "Light" } else { "Dark" };
                        if secondary_button(ui, p, icon).clicked() {
                            self.dark = !self.dark;
                            self.theme_dirty = true;
                        }
                    });
                });
            });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::symmetric(14, 6))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::bottom("status")
            .frame(frame)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    // A small status dot, then the message.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 3.0, p.success);
                    ui.label(
                        RichText::new(&self.status)
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                });
            });
    }

    fn left_panel(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::same(12))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::left("files")
            .default_size(248.0)
            .frame(frame)
            .show_inside(ui, |ui| {
                section_title(ui, p, "Files");
                ui.add_space(2.0);
                ui.label(
                    RichText::new(
                        self.project_dir
                            .as_ref()
                            .and_then(|d| d.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "no project open".into()),
                    )
                    .text_style(TextStyle::Small)
                    .color(p.text_secondary),
                );
                ui.add_space(SPACE);

                if self.files.is_empty() {
                    ui.label(
                        RichText::new("No .mview files.")
                            .color(p.text_disabled),
                    );
                }

                let root = self.project_dir.clone();
                let mut to_open: Option<PathBuf> = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for f in &self.files {
                            let selected = self.open_file.as_ref() == Some(f);
                            if file_row(ui, p, &root, f, selected).clicked() {
                                to_open = Some(f.clone());
                            }
                        }
                    });
                if let Some(p) = to_open {
                    self.open(p);
                }
            });
    }

    fn right_panel(&mut self, ui: &mut Ui, p: &Palette) {
        // Event dispatched by a button click this frame (applied after render so
        // we don't borrow `self` while egui borrows the forest).
        let mut clicked: Option<SessionEvent> = None;
        let mut do_reset = false;

        let frame = Frame::new()
            .fill(p.window)
            .inner_margin(Margin::same(12))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::right("preview")
            .default_size(440.0)
            .frame(frame)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    section_title(ui, p, "Preview");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("native egui · LIVE replay")
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
                    });
                });

                // Live-session controls.
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let has_preview = matches!(&self.preview, Some(pr) if pr.ok);
                    ui.add_enabled_ui(has_preview && !self.session.is_empty(), |ui| {
                        if secondary_button(ui, p, "Reset").clicked() {
                            do_reset = true;
                        }
                    });
                    if !self.session.is_empty() {
                        chip(
                            ui,
                            p,
                            &format!("{} event(s)", self.session.len()),
                            p.info,
                        );
                    } else if has_preview {
                        ui.label(
                            RichText::new("click a button to dispatch")
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
                    }
                });
                ui.add_space(SPACE);

                if let Some(err) = &self.replay_error {
                    ui.colored_label(p.error, "Replay failed (forest unchanged):");
                    ui.label(
                        RichText::new(err)
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                    ui.add_space(SPACE);
                }

                match &self.preview {
                    None => {
                        empty_state(
                            ui,
                            p,
                            "▦",
                            "No preview yet",
                            "Press Preview to render the page IR as a native board.",
                        );
                    }
                    Some(pr) if !pr.ok => {
                        ui.colored_label(p.error, "Preview failed.");
                        if let Some(n) = &pr.note {
                            ui.label(
                                RichText::new(n)
                                    .text_style(TextStyle::Small)
                                    .color(p.text_secondary),
                            );
                        }
                    }
                    Some(pr) => {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                render_forest(ui, p, &pr.forest, &mut clicked);
                            });
                    }
                }
            });

        if do_reset {
            self.reset_session();
        } else if let Some(ev) = clicked {
            self.dispatch_event(ev);
        }
    }

    fn central_editor(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.window)
            .inner_margin(Margin::same(14));
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                // Header row: open file name + dirty marker.
                ui.horizontal(|ui| match &self.open_file {
                    Some(path) => {
                        file_glyph(ui, p);
                        ui.label(
                            RichText::new(path.display().to_string())
                                .size(theme::SUB_HEADING_SIZE)
                                .strong()
                                .color(p.text_primary),
                        );
                        if self.dirty {
                            chip(ui, p, "unsaved", p.warning);
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new("Editor")
                                .size(theme::SUB_HEADING_SIZE)
                                .color(p.text_secondary),
                        );
                    }
                });
                ui.add_space(SPACE);

                // Diagnostics with severity chips.
                if let Some(report) = &self.diagnostics {
                    diagnostics_view(ui, p, report);
                    ui.add_space(SPACE);
                }

                // The editor body.
                if self.open_file.is_none() {
                    empty_state(
                        ui,
                        p,
                        "›",
                        "Select a file",
                        "Pick a .mview file on the left to edit it here.",
                    );
                    return;
                }

                // Code editor inside a card.
                let dark = self.dark;
                let mut layouter =
                    move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                        let mut job = highlight_job(text.as_str(), dark);
                        job.wrap.max_width = wrap_width;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };

                let card = Frame::new()
                    .fill(if p.dark { p.card } else { p.window })
                    .inner_margin(Margin::same(10))
                    .corner_radius(CornerRadius::same(RADIUS_CARD))
                    .stroke(Stroke {
                        width: 1.0,
                        color: p.stroke,
                    });
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        card.show(ui, |ui| {
                            let resp = ui.add(
                                egui::TextEdit::multiline(&mut self.editor_text)
                                    .code_editor()
                                    .frame(Frame::NONE)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(26)
                                    .layouter(&mut layouter),
                            );
                            if resp.changed() {
                                self.dirty = true;
                            }
                        });
                    });
            });
    }
}

// ---------------------------------------------------------------------------
// Reusable Fluent widgets
// ---------------------------------------------------------------------------

/// A 9px brand-colored dot used in the wordmark.
fn brand_dot(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 18.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 5.0, p.brand);
}

/// A thin vertical divider.
fn vsep(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 22.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke {
            width: 1.0,
            color: p.stroke,
        },
    );
}

/// A bold section title in the panel header style.
fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::SUB_HEADING_SIZE)
            .strong()
            .color(p.text_primary),
    );
}

/// A small file glyph (a rounded rect) drawn before file names.
fn file_glyph(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::hover());
    let r = rect.shrink2(egui::vec2(1.0, 0.0));
    ui.painter().rect_filled(
        r,
        CornerRadius::same(2),
        p.text_secondary.gamma_multiply(0.55),
    );
}

/// A pill-shaped colored chip with a label (counts, badges).
fn chip(ui: &mut Ui, p: &Palette, text: &str, accent: Color32) {
    let bg = accent.gamma_multiply(if p.dark { 0.22 } else { 0.16 });
    Frame::new()
        .fill(bg)
        .inner_margin(Margin::symmetric(8, 2))
        .corner_radius(CornerRadius::same(10))
        .stroke(Stroke {
            width: 1.0,
            color: accent.gamma_multiply(0.5),
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .text_style(TextStyle::Small)
                    .strong()
                    .color(accent),
            );
        });
}

/// A PRIMARY (brand-filled) button with on-accent text. Returns its `Response`.
fn primary_button(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    debug_assert!(!label.is_empty(), "primary_button label must be non-empty");
    let fill = if ui.is_enabled() { p.brand } else { p.brand.gamma_multiply(0.5) };
    let btn = egui::Button::new(
        RichText::new(label)
            .text_style(TextStyle::Button)
            .strong()
            .color(p.on_brand),
    )
    .fill(fill)
    .corner_radius(CornerRadius::same(RADIUS_CARD))
    .stroke(Stroke::NONE)
    .min_size(egui::vec2(0.0, 28.0));
    ui.add(btn)
}

/// A SECONDARY (card-fill + hairline) button — the default Fluent style for
/// non-primary actions. Returns its `Response`.
fn secondary_button(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    debug_assert!(!label.is_empty(), "secondary_button label must be non-empty");
    let btn = egui::Button::new(
        RichText::new(label)
            .text_style(TextStyle::Button)
            .color(p.text_primary),
    )
    .fill(p.card)
    .corner_radius(CornerRadius::same(RADIUS_CARD))
    .stroke(Stroke {
        width: 1.0,
        color: p.stroke,
    })
    .min_size(egui::vec2(0.0, 28.0));
    ui.add(btn)
}

/// A tiny labeled action button for deal cards (move / remove). ALWAYS shows a
/// readable label — never an empty glyph square.
fn icon_action_button(ui: &mut Ui, p: &Palette, label: &str, accent: Color32) -> Response {
    debug_assert!(!label.is_empty(), "icon_action_button label must be non-empty");
    let btn = egui::Button::new(
        RichText::new(label)
            .text_style(TextStyle::Small)
            .strong()
            .color(accent),
    )
    .fill(p.raised)
    .corner_radius(CornerRadius::same(RADIUS_INPUT))
    .stroke(Stroke {
        width: 1.0,
        color: p.stroke,
    })
    .min_size(egui::vec2(26.0, 22.0));
    ui.add(btn)
}

/// A selectable file row with hover + selected (brand-subtle) states. The
/// folder portion is dimmed, the filename strong.
fn file_row(
    ui: &mut Ui,
    p: &Palette,
    root: &Option<PathBuf>,
    file: &PathBuf,
    selected: bool,
) -> Response {
    let (folder, name) = split_display(root, file);
    let bg = if selected { p.brand_subtle } else { Color32::TRANSPARENT };
    let stroke = if selected {
        Stroke {
            width: 1.0,
            color: p.brand,
        }
    } else {
        Stroke::NONE
    };

    let resp = Frame::new()
        .fill(bg)
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(CornerRadius::same(RADIUS_INPUT))
        .stroke(stroke)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                file_glyph(ui, p);
                if !folder.is_empty() {
                    ui.label(
                        RichText::new(format!("{folder}/"))
                            .text_style(TextStyle::Small)
                            .color(p.text_disabled),
                    );
                }
                ui.label(
                    RichText::new(name)
                        .strong()
                        .color(if selected { p.text_primary } else { p.text_secondary }),
                );
            });
        })
        .response;

    // Make the whole row clickable + show a hover background.
    let resp = resp.interact(egui::Sense::click());
    if resp.hovered() && !selected {
        ui.painter().rect_filled(
            resp.rect,
            CornerRadius::same(RADIUS_INPUT),
            p.raised.gamma_multiply(0.6),
        );
    }
    resp
}

/// A tasteful empty-state block: a big dim glyph + a title + a hint.
fn empty_state(ui: &mut Ui, p: &Palette, glyph: &str, title: &str, hint: &str) {
    ui.add_space(SPACE * 3.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(glyph)
                .size(40.0)
                .color(p.text_disabled),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(title)
                .size(theme::SUB_HEADING_SIZE)
                .strong()
                .color(p.text_secondary),
        );
        ui.label(
            RichText::new(hint)
                .text_style(TextStyle::Small)
                .color(p.text_disabled),
        );
    });
}

// ---------------------------------------------------------------------------
// Diagnostics view (severity chips + colored rows)
// ---------------------------------------------------------------------------

fn diagnostics_view(ui: &mut Ui, p: &Palette, report: &CommandReport) {
    let errs = report.diagnostics.iter().filter(|d| d.is_error()).count();
    let warns = report.diagnostics.iter().filter(|d| d.is_warning()).count();
    ui.horizontal(|ui| {
        section_title(ui, p, "Diagnostics");
        ui.add_space(4.0);
        chip(ui, p, &format!("{errs} errors"), p.error);
        chip(ui, p, &format!("{warns} warnings"), p.warning);
    });
    ui.add_space(4.0);

    let card = Frame::new()
        .fill(p.card)
        .inner_margin(Margin::same(8))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .stroke(Stroke {
            width: 1.0,
            color: p.stroke,
        });
    card.show(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .auto_shrink([false, false])
            .id_salt("diags")
            .show(ui, |ui| {
                if report.diagnostics.is_empty() {
                    ui.horizontal(|ui| {
                        severity_dot(ui, p.success);
                        ui.label(
                            RichText::new("Clean — no diagnostics.")
                                .color(p.text_secondary),
                        );
                    });
                }
                for d in &report.diagnostics {
                    let color = if d.is_error() {
                        p.error
                    } else if d.is_warning() {
                        p.warning
                    } else {
                        p.info
                    };
                    ui.horizontal_wrapped(|ui| {
                        severity_dot(ui, color);
                        ui.label(
                            RichText::new(format!("[{}]", d.rule))
                                .text_style(TextStyle::Small)
                                .strong()
                                .color(color),
                        );
                        let loc = if d.line > 0 {
                            format!("{}:{}:{}", d.file, d.line, d.col)
                        } else {
                            d.file.clone()
                        };
                        if !loc.is_empty() {
                            ui.label(
                                RichText::new(loc)
                                    .monospace()
                                    .color(p.text_disabled),
                            );
                        }
                        ui.label(RichText::new(&d.message).color(p.text_primary));
                    });
                }
            });
    });
}

/// A small filled severity dot.
fn severity_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

// ---------------------------------------------------------------------------
// Syntax highlighting layouter (uses the pure classifier)
// ---------------------------------------------------------------------------

fn class_color(class: TokenClass, dark: bool) -> Color32 {
    if dark {
        match class {
            TokenClass::Plain => Color32::from_rgb(0xd6, 0xd3, 0xde),
            TokenClass::Directive => Color32::from_rgb(0xc8, 0x9c, 0xff), // brand-ish purple
            TokenClass::Tag => Color32::from_rgb(0x6f, 0xb6, 0xff),       // blue
            TokenClass::StringLit => Color32::from_rgb(0xce, 0x91, 0x78), // orange
            TokenClass::Comment => Color32::from_rgb(0x7e, 0xa6, 0x6f),   // green
        }
    } else {
        match class {
            TokenClass::Plain => Color32::from_rgb(0x28, 0x28, 0x28),
            TokenClass::Directive => Color32::from_rgb(0x6d, 0x28, 0xd9),
            TokenClass::Tag => Color32::from_rgb(0x12, 0x55, 0xc0),
            TokenClass::StringLit => Color32::from_rgb(0xa0, 0x3c, 0x00),
            TokenClass::Comment => Color32::from_rgb(0x2f, 0x7a, 0x33),
        }
    }
}

fn highlight_job(text: &str, dark: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let font = FontId::monospace(13.0);
    let mut rest = text;
    while !rest.is_empty() {
        let (line, tail, had_nl) = match rest.find('\n') {
            Some(pos) => (&rest[..pos], &rest[pos + 1..], true),
            None => (rest, "", false),
        };
        for span in classify_line(line) {
            let fmt = TextFormat::simple(font.clone(), class_color(span.class, dark));
            job.append(&line[span.start..span.end], 0.0, fmt);
        }
        if had_nl {
            job.append(
                "\n",
                0.0,
                TextFormat::simple(font.clone(), class_color(TokenClass::Plain, dark)),
            );
        }
        rest = tail;
    }
    job
}

// ---------------------------------------------------------------------------
// The NATIVE IR renderer — a Fluent KANBAN board for the CRM, graceful
// fallback for arbitrary forests.
// ---------------------------------------------------------------------------

/// Does this node carry the given class token (space-separated `class` attr)?
fn has_class(node: &UiNode, class: &str) -> bool {
    if let UiNode::El { attrs, .. } = node {
        if let Some(cls) = attrs.get("class") {
            return cls.split_whitespace().any(|c| c == class);
        }
    }
    false
}

/// First descendant (DFS) carrying `class`.
fn find_class<'a>(node: &'a UiNode, class: &str) -> Option<&'a UiNode> {
    if has_class(node, class) {
        return Some(node);
    }
    if let UiNode::El { children, .. } = node {
        for c in children {
            if let Some(found) = find_class(c, class) {
                return Some(found);
            }
        }
    }
    None
}

/// All descendants (DFS, pre-order) carrying `class`.
fn find_all_class<'a>(node: &'a UiNode, class: &str, out: &mut Vec<&'a UiNode>) {
    if has_class(node, class) {
        out.push(node);
    }
    if let UiNode::El { children, .. } = node {
        for c in children {
            find_all_class(c, class, out);
        }
    }
}

/// Render the whole forest. If it looks like the CRM board (has `kanban-col`
/// columns), draw the polished Fluent Kanban; otherwise fall back to the
/// generic recursive renderer so arbitrary pages still render.
fn render_forest(ui: &mut Ui, p: &Palette, forest: &[UiNode], clicked: &mut Option<SessionEvent>) {
    let mut columns: Vec<&UiNode> = Vec::new();
    let mut header: Option<&UiNode> = None;
    for node in forest {
        find_all_class(node, "kanban-col", &mut columns);
        if header.is_none() {
            header = find_class(node, "crm-header");
        }
    }

    if !columns.is_empty() {
        // Board header (Pipeline title + sub + "+ New deal").
        if let Some(h) = header {
            render_crm_header(ui, p, h, clicked);
            ui.add_space(SPACE);
        }
        // Horizontal row of column cards.
        ui.horizontal_top(|ui| {
            for col in &columns {
                render_kanban_column(ui, p, col, clicked);
                ui.add_space(SPACE);
            }
        });
        return;
    }

    // Generic fallback.
    for node in forest {
        render_node(ui, p, node, clicked);
    }
}

/// The CRM board header: the "Pipeline" title, the sub-line (open deals · value),
/// and the primary "+ New deal" button.
fn render_crm_header(ui: &mut Ui, p: &Palette, node: &UiNode, clicked: &mut Option<SessionEvent>) {
    let title = find_class_or_tag(node, "", "h1")
        .map(backend::node_text)
        .unwrap_or_else(|| "Pipeline".to_string());
    let sub = find_class(node, "crm-sub")
        .map(backend::node_text)
        .unwrap_or_default();

    let card = Frame::new()
        .fill(p.card)
        .inner_margin(Margin::symmetric(14, 12))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .stroke(Stroke {
            width: 1.0,
            color: p.stroke,
        });
    card.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(title)
                        .text_style(TextStyle::Heading)
                        .strong()
                        .color(p.text_primary),
                );
                if !sub.is_empty() {
                    ui.label(
                        RichText::new(sub)
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The "+ New deal" primary button (mv-btn-primary).
                if let Some(btn) = find_button_with_handler(node, "toggleCreate") {
                    let label = nonempty_label(&backend::node_text(btn), "+ New deal");
                    if primary_button(ui, p, &label).clicked() {
                        queue_click(btn, clicked);
                    }
                }
            });
        });
    });
}

/// One Kanban column card: a header (stage name + count pill) + its deal cards.
fn render_kanban_column(
    ui: &mut Ui,
    p: &Palette,
    col: &UiNode,
    clicked: &mut Option<SessionEvent>,
) {
    let title = find_class(col, "kanban-col-title")
        .map(backend::node_text)
        .unwrap_or_default();
    let count = find_class(col, "kanban-col-count")
        .map(backend::node_text)
        .unwrap_or_default();

    let mut cards: Vec<&UiNode> = Vec::new();
    find_all_class(col, "deal-card", &mut cards);

    let col_frame = Frame::new()
        .fill(p.panel)
        .inner_margin(Margin::same(10))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .stroke(Stroke {
            width: 1.0,
            color: p.stroke,
        })
        .shadow(theme::card_shadow(p));

    // The board lays columns out in a HORIZONTAL row, so the column body must
    // be forced top-down (an explicit vertical layout) or the header + cards
    // would flow sideways. We allocate a fixed-width vertical region for it.
    const COL_W: f32 = 232.0;
    col_frame.show(ui, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(COL_W, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(COL_W);
                // Column header.
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(nonempty_label(&title, "Stage"))
                            .size(theme::SUB_HEADING_SIZE)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !count.is_empty() {
                            chip(ui, p, &count, p.brand);
                        }
                    });
                });
                ui.add_space(SPACE);

                if cards.is_empty() {
                    ui.label(
                        RichText::new("No deals")
                            .text_style(TextStyle::Small)
                            .color(p.text_disabled),
                    );
                }
                for card in &cards {
                    render_deal_card(ui, p, card, clicked);
                    ui.add_space(SPACE);
                }
            },
        );
    });
}

/// One deal card: title (semibold) + remove button, company (dim), a value chip,
/// the contact, and a row of move action buttons. EVERY button shows a readable
/// label — no empty glyph squares.
fn render_deal_card(ui: &mut Ui, p: &Palette, card: &UiNode, clicked: &mut Option<SessionEvent>) {
    let title = find_class(card, "deal-title")
        .map(backend::node_text)
        .unwrap_or_default();
    let company = find_class(card, "deal-company")
        .map(backend::node_text)
        .unwrap_or_default();
    let value = find_class(card, "deal-value")
        .map(backend::node_text)
        .unwrap_or_default();
    let contact = find_class(card, "deal-contact")
        .map(backend::node_text)
        .unwrap_or_default();

    let frame = Frame::new()
        .fill(p.card)
        .inner_margin(Margin::same(10))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .stroke(Stroke {
            width: 1.0,
            color: p.stroke,
        });
    frame.show(ui, |ui| {
      let w = ui.available_width().max(180.0);
      ui.allocate_ui_with_layout(egui::vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(w);
        // Title row + remove (✕ -> a labeled "Del" button).
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(nonempty_label(&title, "Untitled"))
                    .size(theme::SUB_HEADING_SIZE)
                    .strong()
                    .color(p.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(btn) = find_button_with_handler(card, "removeDeal") {
                    if icon_action_button(ui, p, "Del", p.error).clicked() {
                        queue_click(btn, clicked);
                    }
                }
            });
        });

        if !company.is_empty() {
            ui.label(
                RichText::new(company)
                    .text_style(TextStyle::Small)
                    .color(p.text_secondary),
            );
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if !value.is_empty() {
                chip(ui, p, &value, p.brand);
            }
            if !contact.is_empty() {
                ui.label(
                    RichText::new(contact)
                        .text_style(TextStyle::Small)
                        .color(p.text_secondary),
                );
            }
        });

        ui.add_space(SPACE);
        // Move controls (◀ / ▶ -> "Back" / "Fwd" labeled buttons).
        ui.horizontal(|ui| {
            let moves = find_buttons_with_handler(card, "moveBack");
            if let Some(btn) = moves.first() {
                if icon_action_button(ui, p, "Back", p.text_secondary).clicked() {
                    queue_click(btn, clicked);
                }
            }
            let fwd = find_buttons_with_handler(card, "moveFwd");
            if let Some(btn) = fwd.first() {
                if icon_action_button(ui, p, "Fwd", p.brand).clicked() {
                    queue_click(btn, clicked);
                }
            }
        });
      });
    });
}

/// Queue a click event for replay dispatch (records the first click per frame).
fn queue_click(node: &UiNode, clicked: &mut Option<SessionEvent>) {
    if clicked.is_none() {
        if let Some(ev) = backend::event_from_node(node) {
            *clicked = Some(ev);
        }
    }
}

/// Return `label` if non-empty (trimmed), else `fallback`. Guarantees no button
/// or title ever renders blank.
fn nonempty_label(label: &str, fallback: &str) -> String {
    let t = label.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

/// Find the first `<button>` (DFS) whose click handler equals `handler`.
fn find_button_with_handler<'a>(node: &'a UiNode, handler: &str) -> Option<&'a UiNode> {
    fn walk<'a>(n: &'a UiNode, handler: &str) -> Option<&'a UiNode> {
        if let UiNode::El { events, children, .. } = n {
            if events.get("click").map(String::as_str) == Some(handler) {
                return Some(n);
            }
            for c in children {
                if let Some(found) = walk(c, handler) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(node, handler)
}

/// All `<button>`s (DFS) whose click handler equals `handler`.
fn find_buttons_with_handler<'a>(node: &'a UiNode, handler: &str) -> Vec<&'a UiNode> {
    let mut out = Vec::new();
    fn walk<'a>(n: &'a UiNode, handler: &str, out: &mut Vec<&'a UiNode>) {
        if let UiNode::El { events, children, .. } = n {
            if events.get("click").map(String::as_str) == Some(handler) {
                out.push(n);
            }
            for c in children {
                walk(c, handler, out);
            }
        }
    }
    walk(node, handler, &mut out);
    out
}

/// First descendant carrying `class`, or — if `class` is empty — the first
/// descendant whose tag matches `tag`.
fn find_class_or_tag<'a>(node: &'a UiNode, class: &str, tag: &str) -> Option<&'a UiNode> {
    if !class.is_empty() {
        return find_class(node, class);
    }
    fn walk<'a>(n: &'a UiNode, tag: &str) -> Option<&'a UiNode> {
        if n.tag() == Some(tag) {
            return Some(n);
        }
        if let UiNode::El { children, .. } = n {
            for c in children {
                if let Some(f) = walk(c, tag) {
                    return Some(f);
                }
            }
        }
        None
    }
    walk(node, tag)
}

/// Generic recursive renderer for arbitrary (non-CRM) forests. The widget
/// CHOICE is the pure, tested `widget_kind`; this fn performs the chosen widget
/// with Fluent styling. EVERY button shows a readable label.
fn render_node(ui: &mut Ui, p: &Palette, node: &UiNode, clicked: &mut Option<SessionEvent>) {
    match backend::widget_kind(node) {
        WidgetKind::Label => {
            if let UiNode::Text { value } = node {
                if !value.trim().is_empty() {
                    ui.label(RichText::new(value).color(p.text_primary));
                }
            }
        }
        WidgetKind::RawLabel => {
            if let UiNode::Raw { html } = node {
                let txt = backend::strip_html(html);
                if !txt.trim().is_empty() {
                    ui.label(
                        RichText::new(txt)
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                }
            }
        }
        WidgetKind::Button => {
            let label = nonempty_label(&backend::node_text(node), "Action");
            if secondary_button(ui, p, &label).clicked() {
                queue_click(node, clicked);
            }
        }
        WidgetKind::Input => {
            if let UiNode::El { tag, attrs, .. } = node {
                let name = attrs.get("name").cloned().unwrap_or_else(|| tag.clone());
                let mut placeholder = attrs.get("placeholder").cloned().unwrap_or_default();
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut placeholder).hint_text(name),
                );
            }
        }
        WidgetKind::Inline => {
            if let UiNode::El { tag, children, .. } = node {
                let is_heading =
                    matches!(tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                if is_heading {
                    let t = backend::node_text(node);
                    if !t.is_empty() {
                        ui.label(
                            RichText::new(t)
                                .size(theme::SUB_HEADING_SIZE)
                                .strong()
                                .color(p.text_primary),
                        );
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for c in children {
                            render_node(ui, p, c, clicked);
                        }
                    });
                }
            }
        }
        WidgetKind::Group | WidgetKind::Unknown => {
            if let UiNode::El { children, .. } = node {
                // Skip whitespace-only groups to avoid stacks of empty boxes.
                let meaningful = children.iter().any(|c| !is_blank(c));
                if !meaningful {
                    return;
                }
                let frame = Frame::new()
                    .fill(p.card)
                    .inner_margin(Margin::same(8))
                    .corner_radius(CornerRadius::same(RADIUS_CARD))
                    .stroke(Stroke {
                        width: 1.0,
                        color: p.stroke,
                    });
                frame.show(ui, |ui| {
                    ui.vertical(|ui| {
                        for c in children {
                            render_node(ui, p, c, clicked);
                        }
                    });
                });
            }
        }
    }
}

/// True for whitespace-only text/raw leaves (the IR is full of indentation raw
/// nodes; rendering them as boxes is the old "stacked gray boxes" problem).
fn is_blank(node: &UiNode) -> bool {
    match node {
        UiNode::Text { value } => value.trim().is_empty(),
        UiNode::Raw { html } => backend::strip_html(html).trim().is_empty(),
        UiNode::El { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// Split a file path into (relative folder, filename) for the file row.
fn split_display(root: &Option<PathBuf>, file: &PathBuf) -> (String, String) {
    let rel: PathBuf = root
        .as_ref()
        .and_then(|r| file.strip_prefix(r).ok().map(|x| x.to_path_buf()))
        .unwrap_or_else(|| {
            file.file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| file.clone())
        });
    let name = rel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel.display().to_string());
    let folder = rel
        .parent()
        .map(|pp| pp.display().to_string())
        .unwrap_or_default();
    (folder, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::parse_forest;
    use std::path::Path;

    /// Load the committed CRM forest fixture (the exact UI-IR `motoview preview
    /// examples/crm` emits) into a previewed app — GPU-free.
    fn app_with_crm_fixture() -> StudioApp {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crm.forest.json");
        let json = backend::read_file(&path).expect("read CRM fixture");
        let forest = parse_forest(json.trim()).expect("parse CRM forest");
        let mut app = StudioApp::new_headless();
        app.preview = Some(PreviewResult {
            forest,
            raw_json: String::new(),
            note: None,
            ok: true,
        });
        app
    }

    #[test]
    fn board_summary_counts_crm_board() {
        let app = app_with_crm_fixture();
        let b = app.board_summary();
        assert_eq!(b.columns, 4, "CRM has four kanban columns");
        assert_eq!(b.deal_cards, 6, "CRM has six deal cards");
        // 6 cards × (remove + back + fwd) = 18 wired action buttons.
        assert_eq!(b.action_buttons, 18, "one remove/back/fwd per deal card");
    }

    #[test]
    fn no_board_button_renders_empty() {
        // The empty-square regression guard: every interactive board button
        // resolves to a non-empty label (the "+ New deal" header button and the
        // fixed-label per-card Del/Back/Fwd buttons).
        let app = app_with_crm_fixture();
        assert_eq!(
            app.board_summary().empty_button_labels,
            0,
            "no preview button may render as an empty square"
        );
    }

    #[test]
    fn nonempty_label_falls_back() {
        assert_eq!(nonempty_label("  ", "fallback"), "fallback");
        assert_eq!(nonempty_label("  Hi ", "fallback"), "Hi");
    }
}
