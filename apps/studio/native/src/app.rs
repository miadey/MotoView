//! The egui/eframe GUI shell — a Fluent 2 VISUAL DESIGNER for MotoView.
//!
//! Deliberately THIN: every decision of substance (which widget renders a node,
//! how to parse diagnostics, file IO, spawning the compiler) lives in `backend`
//! / `highlight` and is unit-tested headless. The LOOK comes from
//! [`crate::theme`]: a Fluent token foundation (brand = MotoView `#6d28d9`).
//!
//! Layout (a Figma/VB-style designer, all on the Fluent tokens):
//!   * top    : wordmark + brand dot, project-path input + Open, an action
//!              cluster (Check / Lint / Preview / Save), the route field, and a
//!              dark/light toggle — THEN a design toolbar: a [Design | Code] view
//!              toggle, a [Desktop | Web | Mobile] device switcher, and a grid
//!              on/off toggle (active segments use the brand-subtle selection).
//!   * left   : the component TOOLBOX — titled palette sections (Layout / Basic /
//!              Secure) of draggable-looking rows, with a collapsible Pages/Files
//!              list underneath. (Drag-drop is the NEXT slice; this is the visual
//!              palette.)
//!   * center : the DESIGN CANVAS — a scrollable surface with a painted designer
//!              grid and a centered DEVICE FRAME (phone bezel / faux browser /
//!              plain window) at the selected width; the app renders inside it via
//!              the existing `render_forest` (selection still highlights). The
//!              Code view swaps in the syntax-highlighted editor here instead. A
//!              slim diagnostics strip lives at the bottom.
//!   * right  : the INSPECTOR — the selected node's tag, source location, ATTRS
//!              (name = value) and EVENTS (event -> handler). Read-only this slice
//!              (editing is the NEXT one); an empty state when nothing is selected.
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

/// The device frame the design canvas renders the app inside. The chosen
/// variant fixes the inner content width (a responsive preview) and the chrome
/// painted around it (a phone bezel / a faux browser / a plain window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// A plain desktop window frame at ~1280px.
    Desktop,
    /// A faux browser (3 dots + an address pill) at ~1024px.
    Web,
    /// A phone bezel at ~390px.
    Mobile,
}

impl Device {
    /// The inner content width the app forest is constrained to.
    pub fn content_width(self) -> f32 {
        match self {
            Device::Desktop => 1280.0,
            Device::Web => 1024.0,
            Device::Mobile => 390.0,
        }
    }
    /// The on-canvas frame width (content + the chrome's side padding). The
    /// canvas centers a frame this wide; an oversized desktop frame is allowed
    /// to exceed the viewport (the canvas scrolls horizontally).
    fn frame_width(self) -> f32 {
        self.content_width() + 32.0
    }
    fn label(self) -> &'static str {
        match self {
            Device::Desktop => "Desktop",
            Device::Web => "Web",
            Device::Mobile => "Mobile",
        }
    }
}

/// Which surface the CENTER shows: the visual DESIGN canvas (default) or the
/// raw CODE editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignView {
    Design,
    Code,
}

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
/// One editable row of the property grid: an attribute parsed from the `.mview`
/// source at its span. `editable` is false for `{expr}`/concat/bind/bool attrs
/// (shown read-only so they are never corrupted).
#[derive(Debug, Clone)]
struct AttrFieldUi {
    name: String,
    value: String,
    editable: bool,
    quote: char,
    span: backend::SrcSpan,
    file: String,
}

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

    // --- GUI selection (the designer canvas) ------------------------------
    /// The `data-mv-src` id of the currently-selected preview node, resolved to a
    /// source location via [`PreviewResult::srcmap`]. `None` = nothing selected.
    selected_src: Option<usize>,
    /// Canvas mode: `true` = SELECT (designer — clicking a node highlights it +
    /// shows its source; buttons select instead of fire); `false` = INTERACT
    /// (LIVE replay — clicking a button dispatches its event).
    select_mode: bool,
    /// The selected node's editable attribute fields (the property grid), parsed
    /// from the `.mview` source at each attr's span. Recomputed only when the
    /// selection changes (so in-progress edits aren't clobbered each frame).
    inspector_fields: Vec<AttrFieldUi>,
    /// The `selected_src` the `inspector_fields` were computed for (recompute when
    /// it differs, or when forced to `None` after an edit shifts the spans).
    inspector_for: Option<usize>,

    // --- designer layout (the visual designer) ----------------------------
    /// The device frame the design canvas renders the app inside (changes the
    /// responsive content width + the painted chrome).
    device: Device,
    /// Center surface: the visual DESIGN canvas (default) or the CODE editor.
    view: DesignView,
    /// Whether the designer grid background is painted behind the device frame.
    grid: bool,
    /// Whether the collapsible Pages/Files list under the toolbox is expanded.
    files_open: bool,
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
            // LIGHT by default — the bright Canva look is the studio's resting
            // state (the Dark/Light toggle still flips to dark on demand).
            dark: false,
            theme_dirty: true,
            session: Vec::new(),
            session_route: None,
            replay_error: None,
            selected_src: None,
            select_mode: true,
            inspector_fields: Vec::new(),
            inspector_for: None,
            device: Device::Desktop,
            view: DesignView::Design,
            grid: true,
            files_open: false,
        }
    }
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Install the Fluent LIGHT theme immediately so the very first frame is
        // already on-brand and bright (the Canva default; no flash of default
        // egui chrome).
        theme::apply_fluent_theme(&cc.egui_ctx, false);

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

    /// Select the canvas device frame (Desktop / Web / Mobile). Changes the
    /// responsive content width the preview is constrained to and the chrome
    /// painted around it. A designer/test setter (the device switcher).
    pub fn set_device(&mut self, device: Device) {
        self.device = device;
    }

    /// The current canvas device frame.
    pub fn device(&self) -> Device {
        self.device
    }

    /// Switch the center surface between the visual Design canvas and the Code
    /// editor.
    pub fn set_view(&mut self, view: DesignView) {
        self.view = view;
    }

    /// Toggle the designer grid background behind the device frame.
    pub fn set_grid(&mut self, on: bool) {
        self.grid = on;
    }

    /// Select the first preview node carrying `class` (a designer/test helper for
    /// the selection bridge). Returns true if a source-mapped node was selected.
    /// Because a `@for` template shares ONE source id, selecting e.g. `deal-card`
    /// highlights every rendered instance — the template's footprint.
    pub fn select_node_by_class(&mut self, class: &str) -> bool {
        fn find(nodes: &[UiNode], class: &str) -> Option<usize> {
            for n in nodes {
                if let UiNode::El { attrs, children, .. } = n {
                    let hit = attrs
                        .get("class")
                        .map(|c| c.split_whitespace().any(|w| w == class))
                        .unwrap_or(false);
                    if hit {
                        if let Some(id) = backend::node_src_id(n) {
                            return Some(id);
                        }
                    }
                    if let Some(id) = find(children, class) {
                        return Some(id);
                    }
                }
            }
            None
        }
        let id = self.preview.as_ref().and_then(|pr| find(&pr.forest, class));
        self.selected_src = id;
        id.is_some()
    }

    /// The selected node's source location (`file:line:col  <tag>`), resolved
    /// through the preview side-map. `None` if nothing is selected or unresolved.
    pub fn selected_source_location(&self) -> Option<String> {
        let id = self.selected_src?;
        let e = self.preview.as_ref()?.srcmap.get(id)?;
        Some(format!("{}:{}:{}  <{}>", e.file, e.span.line, e.span.col, e.tag))
    }

    /// Drop `kind` into the first preview node carrying `class` — the same path a
    /// canvas drag-drop takes (resolve target -> add_component -> re-preview). A
    /// designer/test helper. Returns true if a target was found + applied.
    pub fn drop_component_into_class(&mut self, class: &str, kind: backend::ComponentKind) -> bool {
        if self.select_node_by_class(class) {
            if let Some(id) = self.selected_src {
                self.add_component_at(id, kind);
                return true;
            }
        }
        false
    }

    /// The first forest node whose `data-mv-src` id equals the current selection
    /// (the node the inspector reads attrs/events from). `@for`-expanded
    /// instances share the template id, so this returns the first instance.
    fn selected_node(&self) -> Option<&UiNode> {
        let id = self.selected_src?;
        let pr = self.preview.as_ref()?;
        fn find<'a>(nodes: &'a [UiNode], id: usize) -> Option<&'a UiNode> {
            for n in nodes {
                if backend::node_src_id(n) == Some(id) {
                    return Some(n);
                }
                if let UiNode::El { children, .. } = n {
                    if let Some(f) = find(children, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        find(&pr.forest, id)
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
                    // Replay (post-dispatch) doesn't emit a side-map in this slice;
                    // selection resolves on the initial preview. Follow-up: thread
                    // --srcmap through replay for post-click re-selection.
                    srcmap: backend::SrcMap::default(),
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

        // TOP: wordmark + actions, then the design toolbar (view / device / grid).
        self.top_bar(ui, &p);
        self.design_toolbar(ui, &p);
        // BOTTOM: a slim status strip (the diagnostics live under the canvas).
        self.bottom_bar(ui, &p);
        // LEFT = the component TOOLBOX (with a collapsible Pages/Files list).
        self.toolbox_panel(ui, &p);
        // RIGHT = the INSPECTOR (selected node's tag / source / attrs / events).
        self.inspector_panel(ui, &p);
        // CENTER = the DESIGN CANVAS (device frame on a grid) — or the Code editor.
        self.center_panel(ui, &p);
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
        let select_mode = self.select_mode;
        let selected = self.selected_src;
        frame.show(ui, |ui| {
            section_title(ui, &p, "Preview · Kanban board");
            ui.add_space(SPACE);
            let mut sink: Option<SessionEvent> = None;
            let mut sel = SelCtx {
                select_mode,
                selected,
                picked: None,
                dropped: None,
            };
            match &self.preview {
                Some(pr) if pr.ok => render_forest(ui, &p, &pr.forest, &mut sink, &mut sel),
                _ => empty_state(ui, &p, "▦", "No preview", "Load a project and run preview."),
            }
        });
    }

    fn top_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::symmetric(18, 12))
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

    /// The DESIGN TOOLBAR (a second top row): a [Design | Code] view toggle, a
    /// [Desktop | Web | Mobile] device switcher, and a grid on/off toggle. The
    /// active segment uses the brand-subtle selection (via `seg_button`).
    fn design_toolbar(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::symmetric(18, 10))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::top("design-toolbar")
            .frame(frame)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    // [Design | Code] view toggle.
                    ui.label(
                        RichText::new("View")
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                    segmented(ui, |ui| {
                        if seg_button(ui, p, "Design", self.view == DesignView::Design).clicked() {
                            self.view = DesignView::Design;
                        }
                        if seg_button(ui, p, "Code", self.view == DesignView::Code).clicked() {
                            self.view = DesignView::Code;
                        }
                    });

                    group_gap(ui, p);

                    // [Desktop | Web | Mobile] device switcher.
                    ui.label(
                        RichText::new("Device")
                            .text_style(TextStyle::Small)
                            .color(p.text_secondary),
                    );
                    segmented(ui, |ui| {
                        for d in [Device::Desktop, Device::Web, Device::Mobile] {
                            if seg_button(ui, p, d.label(), self.device == d).clicked() {
                                self.device = d;
                            }
                        }
                    });

                    group_gap(ui, p);

                    // Grid on/off toggle + the live content width readout.
                    if seg_button(ui, p, "Grid", self.grid).clicked() {
                        self.grid = !self.grid;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        chip(
                            ui,
                            p,
                            &format!("{:.0}px", self.device.content_width()),
                            p.brand,
                        );
                        ui.label(
                            RichText::new(self.device.label())
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
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

    /// LEFT = the component TOOLBOX: titled palette sections (Layout / Basic /
    /// Secure) of draggable-looking rows, with a collapsible Pages/Files list
    /// underneath. Drag-drop is NOT wired this slice — this is the visual palette
    /// (the next slice makes the rows drop onto the canvas).
    fn toolbox_panel(&mut self, ui: &mut Ui, p: &Palette) {
        // Roomier inner margin so the toolbox breathes (Canva whitespace).
        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::same(18))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::left("toolbox")
            .default_size(284.0)
            .frame(frame)
            .show_inside(ui, |ui| {
                section_title(ui, p, "Elements");
                ui.label(
                    RichText::new("drag a component onto the canvas")
                        .text_style(TextStyle::Small)
                        .color(p.text_secondary),
                );
                ui.add_space(SPACE + 2.0);

                // A search-looking field (a Canva touch). Non-functional
                // placeholder this slice — labeled so it never lies.
                search_field(ui, p);
                ui.add_space(SPACE + 4.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("toolbox-scroll")
                    .show(ui, |ui| {
                        // Layout primitives — a friendly 2-column tile grid.
                        palette_section(ui, p, "Layout", false, &[
                            ("▭", "Container"),
                            ("≡", "Row"),
                            ("‖", "Column"),
                        ]);
                        ui.add_space(SPACE * 2.0);
                        // Basic widgets.
                        palette_section(ui, p, "Basic", false, &[
                            ("T", "Text"),
                            ("◉", "Button"),
                            ("▢", "Input"),
                            ("◭", "Image"),
                        ]);
                        ui.add_space(SPACE * 2.0);
                        // SECURE primitives — visually distinct (brand accent + lock).
                        palette_section(ui, p, "Secure", true, &[
                            ("⚿", "Secure form"),
                            ("⚿", "Encrypted field"),
                            ("⚿", "Wallet button"),
                            ("⚿", "II auth"),
                        ]);

                        ui.add_space(SPACE * 2.0);
                        vsep_h(ui, p);
                        ui.add_space(SPACE * 2.0);

                        // The collapsible Pages/Files list (secondary now that the
                        // toolbox is the primary left content).
                        self.files_open = collapsible_header(
                            ui,
                            p,
                            "Pages & files",
                            self.files_open,
                            self.files.len(),
                        );
                        if self.files_open {
                            ui.add_space(4.0);
                            if self.files.is_empty() {
                                ui.label(
                                    RichText::new("No .mview files.")
                                        .text_style(TextStyle::Small)
                                        .color(p.text_disabled),
                                );
                            }
                            let root = self.project_dir.clone();
                            let mut to_open: Option<PathBuf> = None;
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for f in &self.files {
                                let selected = self.open_file.as_ref() == Some(f);
                                if file_row(ui, p, &root, f, selected).clicked() {
                                    to_open = Some(f.clone());
                                }
                            }
                            if let Some(path) = to_open {
                                self.open(path);
                            }
                        }
                    });
            });
    }

    /// RIGHT = the INSPECTOR. With a node selected it shows the tag, the source
    /// location, then the node's ATTRIBUTES (name = value) and EVENTS
    /// (event -> handler), read straight from the selected forest node. Read-only
    /// this slice (editing is the next one). Nothing selected -> an empty state.
    /// Rebuild the property-grid fields for the current selection: read the
    /// `.mview` file and parse each attribute at its source span (the selection
    /// bridge gives the spans). Literals become editable; everything else read-only.
    fn recompute_inspector(&mut self) {
        self.inspector_fields.clear();
        let (Some(id), Some(dir)) = (self.selected_src, self.project_dir.clone()) else {
            return;
        };
        let Some(entry) = self.preview.as_ref().and_then(|pr| pr.srcmap.get(id)).cloned() else {
            return;
        };
        let Ok(source) = backend::read_file(&dir.join(&entry.file)) else {
            return;
        };
        for a in &entry.attrs {
            if a.name == "data-mv-src" {
                continue;
            }
            let text = backend::slice_span(&source, &a.span);
            let (name, value, editable, quote) = backend::parse_attr_source(&text);
            if name.is_empty() {
                continue;
            }
            self.inspector_fields.push(AttrFieldUi {
                name,
                value,
                editable,
                quote,
                span: a.span.clone(),
                file: entry.file.clone(),
            });
        }
    }

    /// Apply the i-th property-grid edit: write the attribute (revert on a compile
    /// error), reload the editor if that file is open, and re-preview.
    fn commit_attr_edit(&mut self, i: usize) {
        let Some(f) = self.inspector_fields.get(i).cloned() else {
            return;
        };
        if !f.editable {
            return;
        }
        let Some(dir) = self.project_dir.clone() else {
            return;
        };
        match backend::edit_attr(&dir, &f.file, &f.span, &f.name, &f.value, f.quote) {
            Ok(updated) => {
                self.status = format!("{} = \"{}\"", f.name, f.value);
                let path = dir.join(&f.file);
                if self.open_file.as_deref() == Some(path.as_path()) {
                    self.editor_text = updated;
                    self.dirty = false;
                }
                self.run_preview();
            }
            Err(e) => {
                self.status = format!("Edit reverted: {e}");
            }
        }
        // Spans shifted (or were restored on revert) — rebuild from fresh source.
        self.inspector_for = None;
    }

    fn inspector_panel(&mut self, ui: &mut Ui, p: &Palette) {
        // Recompute the editable fields only when the selection changed (so an
        // in-progress edit isn't clobbered every frame).
        if self.inspector_for != self.selected_src {
            self.recompute_inspector();
            self.inspector_for = self.selected_src;
        }
        let node = self.selected_node().cloned();
        let location = self.selected_source_location();
        let mut commit: Option<usize> = None;

        let frame = Frame::new()
            .fill(p.panel)
            .inner_margin(Margin::same(18))
            .stroke(Stroke {
                width: 1.0,
                color: p.stroke,
            });
        egui::Panel::right("inspector")
            .default_size(320.0)
            .frame(frame)
            .show_inside(ui, |ui| {
                section_title(ui, p, "Inspector");
                ui.add_space(SPACE);

                let Some(node) = node else {
                    empty_state(
                        ui,
                        p,
                        "◇",
                        "Nothing selected",
                        "Select a node on the canvas to inspect it.",
                    );
                    return;
                };

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("inspector-scroll")
                    .show(ui, |ui| {
                        inspector_header(ui, p, &node, location.as_deref());

                        // EDITABLE ATTRIBUTES — the property grid.
                        ui.label(
                            RichText::new("Attributes")
                                .text_style(TextStyle::Small)
                                .strong()
                                .color(p.text_secondary),
                        );
                        ui.add_space(4.0);
                        if self.inspector_fields.is_empty() {
                            ui.label(
                                RichText::new("— none —")
                                    .text_style(TextStyle::Small)
                                    .color(p.text_disabled),
                            );
                        } else {
                            let card = Frame::new()
                                .fill(p.card)
                                .inner_margin(Margin::same(8))
                                .corner_radius(CornerRadius::same(RADIUS_CARD))
                                .stroke(Stroke { width: 1.0, color: p.stroke });
                            card.show(ui, |ui| {
                                for i in 0..self.inspector_fields.len() {
                                    if attr_field_row(ui, p, &mut self.inspector_fields[i]) {
                                        commit = Some(i);
                                    }
                                }
                            });
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new("Edit a literal value, press Enter to apply.")
                                    .text_style(TextStyle::Small)
                                    .color(p.text_disabled),
                            );
                        }

                        ui.add_space(SPACE);
                        inspector_events(ui, p, &node);
                    });
            });

        if let Some(i) = commit {
            self.commit_attr_edit(i);
        }
    }

    /// CENTER = the DESIGN CANVAS (default) or the CODE editor, by `self.view`.
    /// The diagnostics strip lives at the BOTTOM of the central panel so it stays
    /// reachable without dominating the surface.
    fn center_panel(&mut self, ui: &mut Ui, p: &Palette) {
        let frame = Frame::new()
            .fill(p.window)
            .inner_margin(Margin::ZERO);
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                // Diagnostics: a slim BOTTOM strip (reachable, not dominating).
                if let Some(report) = self.diagnostics.clone() {
                    let dframe = Frame::new()
                        .fill(p.panel)
                        .inner_margin(Margin::symmetric(12, 8))
                        .stroke(Stroke {
                            width: 1.0,
                            color: p.stroke,
                        });
                    egui::Panel::bottom("diag-strip")
                        .frame(dframe)
                        .max_size(180.0)
                        .show_inside(ui, |ui| {
                            diagnostics_view(ui, p, &report);
                        });
                }

                match self.view {
                    DesignView::Design => self.design_canvas(ui, p),
                    DesignView::Code => self.code_view(ui, p),
                }
            });
    }

    /// The DESIGN CANVAS: a scrollable surface with a painted designer grid, a
    /// centered DEVICE FRAME at the selected width, and the app rendered inside
    /// it via the existing `render_forest` (selection still works). Click
    /// dispatch (select / live-replay) is collected here and applied after.
    /// Apply a drag-and-drop: insert `kind` as the first child of the node with
    /// `target_id` (resolved through the selection side-map to its `.mview` file +
    /// open-tag end), via backend::add_component (which validates + reverts on a
    /// compile error), then reload the open editor file and re-preview.
    fn add_component_at(&mut self, target_id: usize, kind: backend::ComponentKind) {
        let (file, at) = match self.preview.as_ref().and_then(|pr| pr.srcmap.get(target_id)) {
            Some(e) => (e.file.clone(), e.span.end),
            None => return,
        };
        let Some(dir) = self.project_dir.clone() else {
            return;
        };
        match backend::add_component(&dir, &file, at, kind, "    ") {
            Ok(()) => {
                self.status = format!("Added {} into {}", kind.label(), file);
                self.selected_src = None; // the forest changed; clear stale selection
                let target_path = dir.join(&file);
                if self.open_file.as_deref() == Some(target_path.as_path()) {
                    if let Ok(text) = backend::read_file(&target_path) {
                        self.editor_text = text;
                        self.dirty = false;
                    }
                }
                self.run_preview();
            }
            Err(e) => {
                self.status = format!("Drop failed: {e}");
            }
        }
    }

    fn design_canvas(&mut self, ui: &mut Ui, p: &Palette) {
        let mut clicked: Option<SessionEvent> = None;
        let mut picked: Option<usize> = None;
        let mut dropped: Option<(usize, backend::ComponentKind)> = None;
        let mut do_reset = false;
        let mut set_mode: Option<bool> = None;
        let select_mode = self.select_mode;
        let selected = self.selected_src;
        let device = self.device;
        let grid = self.grid;

        let frame = Frame::new()
            .fill(p.window)
            .inner_margin(Margin::ZERO);
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                // The canvas toolbar: Select/Interact + the live-session controls.
                let bar = Frame::new()
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(18, 10));
                bar.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Canvas mode: Select (designer) vs Interact (LIVE replay).
                        segmented(ui, |ui| {
                            if seg_button(ui, p, "Select", select_mode).clicked() {
                                set_mode = Some(true);
                            }
                            if seg_button(ui, p, "Interact", !select_mode).clicked() {
                                set_mode = Some(false);
                            }
                        });
                        ui.add_space(SPACE);
                        let has_preview = matches!(&self.preview, Some(pr) if pr.ok);
                        ui.add_enabled_ui(has_preview && !self.session.is_empty(), |ui| {
                            if secondary_button(ui, p, "Reset").clicked() {
                                do_reset = true;
                            }
                        });
                        if !self.session.is_empty() {
                            chip(ui, p, &format!("{} event(s)", self.session.len()), p.info);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(if select_mode {
                                    "Select — click a node to inspect it"
                                } else {
                                    "Interact — click a button to dispatch (live replay)"
                                })
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                            );
                        });
                    });
                });

                if let Some(err) = &self.replay_error {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.colored_label(p.error, "Replay failed (forest unchanged):");
                        ui.label(
                            RichText::new(err)
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
                    });
                }

                // The scrollable canvas with the painted grid + the device frame.
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .id_salt("design-canvas")
                    .show(ui, |ui| {
                        // Paint the distinct work-surface (Canva's gray mat)
                        // across the whole canvas FIRST, then the subtle grid on
                        // top — so the white device frame floats on gray, not on
                        // the panel color.
                        ui.painter().rect_filled(
                            ui.clip_rect(),
                            CornerRadius::ZERO,
                            p.canvas,
                        );
                        // Paint the designer grid behind everything in this canvas.
                        if grid {
                            paint_design_grid(ui, p);
                        }
                        // A generous canvas so the grid fills the viewport even when
                        // the device frame is small (mobile).
                        let min = ui.available_size();
                        ui.set_min_size(egui::vec2(
                            min.x.max(device.frame_width() + 140.0),
                            min.y.max(600.0),
                        ));
                        // Comfortable top margin so the floating frame breathes.
                        ui.add_space(48.0);
                        ui.vertical_centered(|ui| {
                            match &self.preview {
                                None => {
                                    device_frame(ui, p, device, |ui| {
                                        empty_state(
                                            ui,
                                            p,
                                            "▦",
                                            "No preview yet",
                                            "Press Preview to render the page on the canvas.",
                                        );
                                    });
                                }
                                Some(pr) if !pr.ok => {
                                    device_frame(ui, p, device, |ui| {
                                        ui.colored_label(p.error, "Preview failed.");
                                        if let Some(n) = &pr.note {
                                            ui.label(
                                                RichText::new(n)
                                                    .text_style(TextStyle::Small)
                                                    .color(p.text_secondary),
                                            );
                                        }
                                    });
                                }
                                Some(pr) => {
                                    let mut sel = SelCtx {
                                        select_mode,
                                        selected,
                                        picked: None,
                                        dropped: None,
                                    };
                                    device_frame(ui, p, device, |ui| {
                                        render_forest(ui, p, &pr.forest, &mut clicked, &mut sel);
                                    });
                                    picked = sel.picked;
                                    dropped = sel.dropped;
                                }
                            }
                        });
                        ui.add_space(48.0);
                    });
            });

        if let Some(m) = set_mode {
            self.select_mode = m;
        }
        if let Some(id) = picked {
            self.selected_src = Some(id);
        }
        if let Some((id, kind)) = dropped {
            self.add_component_at(id, kind);
        }
        if do_reset {
            self.reset_session();
        } else if let Some(ev) = clicked {
            self.dispatch_event(ev);
        }
    }

    /// The CODE view: the existing syntax-highlighted editor in a card. Shown in
    /// the center when the [Design | Code] toggle is on Code.
    fn code_view(&mut self, ui: &mut Ui, p: &Palette) {
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
                            RichText::new("Code")
                                .size(theme::SUB_HEADING_SIZE)
                                .color(p.text_secondary),
                        );
                    }
                });
                ui.add_space(SPACE);

                // The editor body.
                if self.open_file.is_none() {
                    empty_state(
                        ui,
                        p,
                        "›",
                        "Select a file",
                        "Open a .mview file from Pages & files to edit it here.",
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

/// A comfortable gap between toolbar GROUPS: roomy space, a hairline divider,
/// roomy space (more breathing room than a bare `vsep`, the Canva calm).
fn group_gap(ui: &mut Ui, p: &Palette) {
    ui.add_space(SPACE * 1.5);
    vsep(ui, p);
    ui.add_space(SPACE * 1.5);
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

/// A thin HORIZONTAL divider spanning the available width.
fn vsep_h(ui: &mut Ui, p: &Palette) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
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

// ---------------------------------------------------------------------------
// Designer chrome: segmented controls, the toolbox palette, the device frame,
// the painted grid, and the inspector body.
// ---------------------------------------------------------------------------

/// Wrap a row of [`seg_button`]s in a tight group so they read as one segmented
/// control (no gap between segments).
fn segmented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(add);
    });
}

/// One segment of a segmented control. The active segment uses the brand-subtle
/// selection (brand fill + brand text); the inactive ones the card style.
fn seg_button(ui: &mut Ui, p: &Palette, label: &str, active: bool) -> Response {
    debug_assert!(!label.is_empty(), "seg_button label must be non-empty");
    let (fill, text, stroke) = if active {
        (p.brand_subtle, p.brand, p.brand)
    } else {
        (p.card, p.text_secondary, p.stroke)
    };
    let btn = egui::Button::new(
        RichText::new(label)
            .text_style(TextStyle::Button)
            .strong()
            .color(text),
    )
    .fill(fill)
    .corner_radius(CornerRadius::same(RADIUS_INPUT))
    .stroke(Stroke { width: 1.0, color: stroke })
    .min_size(egui::vec2(0.0, 26.0));
    ui.add(btn)
}

/// A subtle, Canva-style SEARCH field at the top of the Elements toolbox. It is
/// a non-functional placeholder this slice (a labeled affordance, not a lie): a
/// rounded input shell with a magnifier glyph + hint text.
fn search_field(ui: &mut Ui, p: &Palette) {
    Frame::new()
        .fill(p.raised)
        .inner_margin(Margin::symmetric(12, 9))
        .corner_radius(CornerRadius::same(10))
        .stroke(Stroke { width: 1.0, color: p.stroke })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("⌕")
                        .size(16.0)
                        .color(p.text_disabled),
                );
                ui.label(
                    RichText::new("Search elements")
                        .color(p.text_disabled),
                );
            });
        });
}

/// A titled toolbox section (Layout / Basic / Secure) rendered as a friendly
/// 2-column TILE GRID (Canva elements) — each tile is a big glyph over a label.
/// `secure` makes the section visually distinct (a brand-tinted tile + a lock
/// glyph header). Drag-drop is NOT wired this slice — the tiles are the visual
/// palette.
fn palette_section(ui: &mut Ui, p: &Palette, title: &str, secure: bool, rows: &[(&str, &str)]) {
    // Section header (a lock glyph for the secure section).
    ui.horizontal(|ui| {
        if secure {
            ui.label(RichText::new("⚿").color(p.brand));
        }
        ui.label(
            RichText::new(title)
                .size(theme::SUB_HEADING_SIZE)
                .strong()
                .color(if secure { p.brand } else { p.text_primary }),
        );
    });
    ui.add_space(SPACE);

    // A 2-column grid of tiles. We compute the tile width from the available
    // width so the two columns + the gap fill the toolbox neatly.
    const GAP: f32 = 10.0;
    let avail = ui.available_width();
    let tile_w = ((avail - GAP) / 2.0).max(96.0);
    let tile_h = 64.0;

    let mut iter = rows.iter().peekable();
    while iter.peek().is_some() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for _ in 0..2 {
                if let Some((glyph, label)) = iter.next() {
                    palette_tile(ui, p, glyph, label, secure, egui::vec2(tile_w, tile_h));
                }
            }
        });
        if iter.peek().is_some() {
            ui.add_space(GAP);
        }
    }
}

/// One palette TILE: a centered big glyph over a small label, in a soft rounded
/// card with a gentle hover. `secure` tints it with the brand accent. The tile
/// is a click-sensed visual affordance (drag-drop is the next slice).
fn palette_tile(
    ui: &mut Ui,
    p: &Palette,
    glyph: &str,
    label: &str,
    secure: bool,
    size: egui::Vec2,
) {
    debug_assert!(!label.is_empty(), "palette_tile label must be non-empty");
    let (id, rect) = ui.allocate_space(size);
    let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
    // DRAG SOURCE: the tile carries its ComponentKind; dropping it on a canvas
    // node inserts the component there (see select_overlay + add_component_at).
    if let Some(kind) = backend::ComponentKind::from_label(label) {
        resp.dnd_set_drag_payload(kind);
        resp.clone().on_hover_cursor(egui::CursorIcon::Grab);
    }

    let hovered = resp.hovered() || resp.dragged();
    let (fill, stroke_c) = if secure {
        (
            if hovered { p.brand_subtle } else { p.brand_subtle.gamma_multiply(0.7) },
            p.brand,
        )
    } else if hovered {
        (p.raised, p.brand)
    } else {
        (p.card, p.stroke)
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS_CARD), fill);
    painter.rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(RADIUS_CARD),
        Stroke { width: 1.0, color: stroke_c },
        egui::StrokeKind::Inside,
    );

    let glyph_color = if secure { p.brand } else { p.text_secondary };
    let center = rect.center();
    painter.text(
        egui::pos2(center.x, rect.top() + size.y * 0.36),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(22.0),
        glyph_color,
    );
    painter.text(
        egui::pos2(center.x, rect.bottom() - size.y * 0.26),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        p.text_primary,
    );
}

/// A clickable collapsible header (a ▸/▾ caret + a title + a count chip). Returns
/// the new open state.
fn collapsible_header(ui: &mut Ui, p: &Palette, title: &str, open: bool, count: usize) -> bool {
    let resp = ui
        .horizontal(|ui| {
            ui.label(
                RichText::new(if open { "▾" } else { "▸" })
                    .text_style(TextStyle::Small)
                    .color(p.text_secondary),
            );
            ui.label(
                RichText::new(title)
                    .text_style(TextStyle::Small)
                    .strong()
                    .color(p.text_secondary),
            );
            chip(ui, p, &count.to_string(), p.brand);
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        !open
    } else {
        open
    }
}

/// Paint a subtle dotted designer grid across the current `ui`'s clip rect, then
/// reserve no space (it's a background). Uses a faint stroke color from the
/// palette so it reads under the device frame without competing with it.
fn paint_design_grid(ui: &mut Ui, p: &Palette) {
    let rect = ui.clip_rect();
    let painter = ui.painter();
    const STEP: f32 = 24.0;
    // A subtle dot that still reads on the gray work surface: on light, a soft
    // cool gray a step darker than the canvas; on dark, the divider color.
    let dot = if p.dark {
        p.stroke.gamma_multiply(0.9)
    } else {
        Color32::from_rgb(0xd2, 0xd4, 0xdd)
    };
    // Cross-dots on a STEP grid: a tiny filled square at each lattice point.
    let mut y = (rect.top() / STEP).floor() * STEP;
    while y < rect.bottom() {
        let mut x = (rect.left() / STEP).floor() * STEP;
        while x < rect.right() {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(1.5, 1.5)),
                CornerRadius::ZERO,
                dot,
            );
            x += STEP;
        }
        y += STEP;
    }
}

/// Render `content` inside a DEVICE FRAME at `device`'s width, centered: a phone
/// bezel (Mobile), a faux browser with 3 dots + an address pill (Web), or a plain
/// window (Desktop). The inner content is constrained to the device's content
/// width so the preview is genuinely responsive.
fn device_frame(ui: &mut Ui, p: &Palette, device: Device, content: impl FnOnce(&mut Ui)) {
    let content_w = device.content_width();
    let frame_w = device.frame_width();

    let outer = Frame::new()
        // The frame is a WHITE card on light (p.card) — white-on-gray + the soft
        // shadow is what makes the design pop like a Canva artboard.
        .fill(p.card)
        .inner_margin(Margin::same(if matches!(device, Device::Mobile) { 12 } else { 0 }))
        .corner_radius(CornerRadius::same(if matches!(device, Device::Mobile) {
            28
        } else {
            10
        }))
        .stroke(Stroke {
            width: if matches!(device, Device::Mobile) { 6.0 } else { 1.0 },
            color: p.stroke,
        })
        // A GENEROUS soft shadow so the white frame floats off the gray canvas
        // (the Canva artboard look) — bigger than a normal card's drop.
        .shadow(theme::frame_shadow(p));

    outer.show(ui, |ui| {
        ui.set_width(frame_w);
        ui.allocate_ui_with_layout(
            egui::vec2(frame_w, 0.0),
            Layout::top_down(Align::Center),
            |ui| {
                ui.set_width(frame_w);
                // Per-device chrome above the content.
                match device {
                    Device::Web => browser_chrome(ui, p, content_w),
                    Device::Desktop => window_chrome(ui, p),
                    Device::Mobile => phone_notch(ui, p),
                }
                // The content surface (the app), constrained to the content width.
                let surface = Frame::new()
                    .fill(p.window)
                    .inner_margin(Margin::same(14))
                    .corner_radius(CornerRadius::same(if matches!(device, Device::Mobile) {
                        18
                    } else {
                        0
                    }));
                surface.show(ui, |ui| {
                    ui.set_width(content_w);
                    ui.allocate_ui_with_layout(
                        egui::vec2(content_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(content_w);
                            ui.set_max_width(content_w);
                            content(ui);
                        },
                    );
                });
            },
        );
    });
}

/// The faux-browser top bar: 3 traffic-light dots + an address pill.
fn browser_chrome(ui: &mut Ui, p: &Palette, content_w: f32) {
    let bar = Frame::new()
        .fill(p.card)
        .inner_margin(Margin::symmetric(12, 8))
        .stroke(Stroke { width: 1.0, color: p.stroke });
    bar.show(ui, |ui| {
        ui.set_width(content_w + 32.0);
        ui.horizontal(|ui| {
            for c in [p.error, p.warning, p.success] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, c);
            }
            ui.add_space(SPACE);
            // The address pill.
            Frame::new()
                .fill(p.window)
                .inner_margin(Margin::symmetric(10, 3))
                .corner_radius(CornerRadius::same(12))
                .stroke(Stroke { width: 1.0, color: p.stroke })
                .show(ui, |ui| {
                    ui.set_width(content_w * 0.6);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⚿").text_style(TextStyle::Small).color(p.success));
                        ui.label(
                            RichText::new("localhost · motoview preview")
                                .text_style(TextStyle::Small)
                                .color(p.text_secondary),
                        );
                    });
                });
        });
    });
}

/// The plain desktop window chrome: a slim title bar with three window dots.
fn window_chrome(ui: &mut Ui, p: &Palette) {
    let bar = Frame::new()
        .fill(p.card)
        .inner_margin(Margin::symmetric(12, 6))
        .stroke(Stroke { width: 1.0, color: p.stroke });
    bar.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            for c in [p.error, p.warning, p.success] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, c);
            }
            ui.add_space(SPACE);
            ui.label(
                RichText::new("MotoView app — desktop")
                    .text_style(TextStyle::Small)
                    .color(p.text_secondary),
            );
        });
    });
}

/// The phone notch: a centered pill at the top of the bezel.
fn phone_notch(ui: &mut Ui, p: &Palette) {
    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        Frame::new()
            .fill(p.card)
            .inner_margin(Margin::symmetric(22, 4))
            .corner_radius(CornerRadius::same(8))
            .show(ui, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(44.0, 5.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(3), p.stroke);
            });
    });
    ui.add_space(4.0);
}

/// The INSPECTOR body for a selected node: the tag, the source location, then
/// its ATTRIBUTES (name = value) and EVENTS (event -> handler). Read-only.
/// Inspector header: the element tag chip + its source location.
fn inspector_header(ui: &mut Ui, p: &Palette, node: &UiNode, location: Option<&str>) {
    let tag = match node {
        UiNode::El { tag, .. } => tag.as_str(),
        UiNode::Text { .. } => "text",
        UiNode::Raw { .. } => "raw",
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("◉").color(p.brand));
        ui.label(
            RichText::new(format!("<{tag}>"))
                .size(theme::SUB_HEADING_SIZE)
                .strong()
                .color(p.text_primary),
        );
    });
    if let Some(loc) = location {
        ui.add_space(2.0);
        ui.label(
            RichText::new(loc)
                .monospace()
                .text_style(TextStyle::Small)
                .color(p.text_secondary),
        );
    }
    ui.add_space(SPACE);
    vsep_h(ui, p);
    ui.add_space(SPACE);
}

/// Inspector EVENTS section (read-only: `event → handler`).
fn inspector_events(ui: &mut Ui, p: &Palette, node: &UiNode) {
    let events = match node {
        UiNode::El { events, .. } => Some(events),
        _ => None,
    };
    ui.label(
        RichText::new("Events")
            .text_style(TextStyle::Small)
            .strong()
            .color(p.text_secondary),
    );
    ui.add_space(4.0);
    let evs: Vec<(&String, &String)> = events.map(|m| m.iter().collect()).unwrap_or_default();
    if evs.is_empty() {
        ui.label(
            RichText::new("— none —")
                .text_style(TextStyle::Small)
                .color(p.text_disabled),
        );
    } else {
        let card = Frame::new()
            .fill(p.card)
            .inner_margin(Margin::same(8))
            .corner_radius(CornerRadius::same(RADIUS_CARD))
            .stroke(Stroke { width: 1.0, color: p.stroke });
        card.show(ui, |ui| {
            for (ev, handler) in evs {
                kv_row(ui, p, ev, handler, "→");
            }
        });
    }
}

/// One EDITABLE property-grid row. A literal attr gets a TextEdit (press Enter to
/// apply); an expr/concat/bind/bool attr is shown read-only (never corrupted).
/// Returns true when the value was committed this frame.
fn attr_field_row(ui: &mut Ui, p: &Palette, f: &mut AttrFieldUi) -> bool {
    let mut committed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&f.name)
                .text_style(TextStyle::Small)
                .strong()
                .color(p.brand),
        );
        ui.label(
            RichText::new("=")
                .text_style(TextStyle::Small)
                .color(p.text_disabled),
        );
        if f.editable {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut f.value)
                    .desired_width(150.0)
                    .font(TextStyle::Small),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                committed = true;
            }
        } else {
            ui.label(
                RichText::new(&f.value)
                    .monospace()
                    .text_style(TextStyle::Small)
                    .color(p.text_disabled),
            );
        }
    });
    committed
}

/// One inspector key/value row: a brand-tinted key, a separator, a mono value.
fn kv_row(ui: &mut Ui, p: &Palette, key: &str, value: &str, sep: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(key)
                .monospace()
                .text_style(TextStyle::Small)
                .strong()
                .color(p.brand),
        );
        ui.label(
            RichText::new(sep)
                .text_style(TextStyle::Small)
                .color(p.text_disabled),
        );
        let shown = if value.is_empty() { "\"\"" } else { value };
        ui.label(
            RichText::new(shown)
                .monospace()
                .text_style(TextStyle::Small)
                .color(p.text_primary),
        );
    });
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
/// Threaded through the preview renderers: the designer SELECTION context. In
/// SELECT mode each source-mapped node is click-to-select with a brand outline; in
/// INTERACT mode buttons dispatch (the LIVE replay path) as before.
struct SelCtx {
    select_mode: bool,
    /// The selected node's `data-mv-src` id (outlined while rendering).
    selected: Option<usize>,
    /// A node clicked for selection THIS frame (out; applied after render).
    picked: Option<usize>,
    /// A component dropped onto a node THIS frame: (target src id, kind). Applied
    /// after render — inserts the component as a child of the target.
    dropped: Option<(usize, backend::ComponentKind)>,
}

/// A button was clicked: SELECT mode selects its source node; INTERACT mode
/// dispatches its event.
fn button_action(
    was_clicked: bool,
    btn: &UiNode,
    sel: &mut SelCtx,
    clicked: &mut Option<SessionEvent>,
) {
    if !was_clicked {
        return;
    }
    if sel.select_mode {
        if let Some(id) = backend::node_src_id(btn) {
            sel.picked = Some(id);
        }
    } else {
        queue_click(btn, clicked);
    }
}

/// In SELECT mode, make an already-rendered node `rect` click-to-select and paint
/// a brand outline when it is the selected node (a faint one on hover). No-op in
/// INTERACT mode or for a node with no source id.
fn select_overlay(ui: &mut Ui, p: &Palette, node: &UiNode, sel: &mut SelCtx, rect: egui::Rect) {
    if !sel.select_mode {
        return;
    }
    let Some(id) = backend::node_src_id(node) else {
        return;
    };
    let resp = ui.interact(
        rect,
        egui::Id::new(("mv-sel", id, rect.left() as i32, rect.top() as i32)),
        egui::Sense::click(),
    );
    if resp.clicked() {
        sel.picked = Some(id);
    }
    // DROP TARGET: a component dragged from the Elements toolbox and released over
    // this node is inserted as its first child. Highlight while a payload hovers.
    if let Some(kind) = resp.dnd_release_payload::<backend::ComponentKind>() {
        sel.dropped = Some((id, *kind));
    }
    let drop_hover = resp.dnd_hover_payload::<backend::ComponentKind>().is_some();
    if drop_hover {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(RADIUS_CARD),
            Stroke { width: 2.0, color: p.brand },
            egui::StrokeKind::Inside,
        );
    } else if sel.selected == Some(id) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(RADIUS_CARD),
            Stroke { width: 2.0, color: p.brand },
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(RADIUS_CARD),
            Stroke { width: 1.0, color: p.text_secondary },
            egui::StrokeKind::Inside,
        );
    }
}

fn render_forest(
    ui: &mut Ui,
    p: &Palette,
    forest: &[UiNode],
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
) {
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
            render_crm_header(ui, p, h, clicked, sel);
            ui.add_space(SPACE);
        }
        // Horizontal row of column cards.
        ui.horizontal_top(|ui| {
            for col in &columns {
                render_kanban_column(ui, p, col, clicked, sel);
                ui.add_space(SPACE);
            }
        });
        return;
    }

    // Generic fallback.
    for node in forest {
        render_node(ui, p, node, clicked, sel);
    }
}

/// The CRM board header: the "Pipeline" title, the sub-line (open deals · value),
/// and the primary "+ New deal" button.
fn render_crm_header(
    ui: &mut Ui,
    p: &Palette,
    node: &UiNode,
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
) {
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
    let r = card.show(ui, |ui| {
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
                    let was = primary_button(ui, p, &label).clicked();
                    button_action(was, btn, sel, clicked);
                }
            });
        });
        // AUTHORED EXTRAS: render any direct-child elements the specialized header
        // above didn't show (e.g. a drag-dropped component) via the generic path,
        // so a drop into the header is VISIBLE + selectable.
        render_extra_children(ui, p, node, clicked, sel, &["h1", "toggleCreate"]);
    });
    select_overlay(ui, p, node, sel, r.response.rect);
}

/// Render the DIRECT child ELEMENTS of `node` that a specialized renderer didn't
/// already show — so a drag-dropped component appears on the canvas. A child is
/// skipped if it (or a descendant) is a known marker: a tag in `skip` (e.g. "h1"
/// for the title wrapper) or a button wired to a handler named in `skip`.
fn render_extra_children(
    ui: &mut Ui,
    p: &Palette,
    node: &UiNode,
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
    skip: &[&str],
) {
    let UiNode::El { children, .. } = node else {
        return;
    };
    for c in children {
        let UiNode::El { tag, events, .. } = c else {
            continue;
        };
        let is_known_wrap = skip
            .iter()
            .any(|k| !k.is_empty() && find_class_or_tag(c, "", k).is_some());
        let is_known_handler = events
            .get("click")
            .map(|h| skip.contains(&h.as_str()))
            .unwrap_or(false)
            || events
                .get("submit")
                .map(|h| skip.contains(&h.as_str()))
                .unwrap_or(false);
        // Skip the structural wrappers the kanban renderer handles itself.
        if is_known_wrap || is_known_handler || tag == "section" {
            continue;
        }
        render_node(ui, p, c, clicked, sel);
    }
}

/// One Kanban column card: a header (stage name + count pill) + its deal cards.
fn render_kanban_column(
    ui: &mut Ui,
    p: &Palette,
    col: &UiNode,
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
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
    let r = col_frame.show(ui, |ui| {
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
                    render_deal_card(ui, p, card, clicked, sel);
                    ui.add_space(SPACE);
                }
            },
        );
    });
    select_overlay(ui, p, col, sel, r.response.rect);
}

/// One deal card: title (semibold) + remove button, company (dim), a value chip,
/// the contact, and a row of move action buttons. EVERY button shows a readable
/// label — no empty glyph squares.
fn render_deal_card(
    ui: &mut Ui,
    p: &Palette,
    card: &UiNode,
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
) {
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
    let r = frame.show(ui, |ui| {
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
                    let was = icon_action_button(ui, p, "Del", p.error).clicked();
                    button_action(was, btn, sel, clicked);
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
                let was = icon_action_button(ui, p, "Back", p.text_secondary).clicked();
                button_action(was, btn, sel, clicked);
            }
            let fwd = find_buttons_with_handler(card, "moveFwd");
            if let Some(btn) = fwd.first() {
                let was = icon_action_button(ui, p, "Fwd", p.brand).clicked();
                button_action(was, btn, sel, clicked);
            }
        });
      });
    });
    select_overlay(ui, p, card, sel, r.response.rect);
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
fn render_node(
    ui: &mut Ui,
    p: &Palette,
    node: &UiNode,
    clicked: &mut Option<SessionEvent>,
    sel: &mut SelCtx,
) {
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
            let was = secondary_button(ui, p, &label).clicked();
            button_action(was, node, sel, clicked);
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
                            render_node(ui, p, c, clicked, sel);
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
                let r = frame.show(ui, |ui| {
                    ui.vertical(|ui| {
                        for c in children {
                            render_node(ui, p, c, clicked, sel);
                        }
                    });
                });
                select_overlay(ui, p, node, sel, r.response.rect);
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
            srcmap: backend::SrcMap::default(),
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
