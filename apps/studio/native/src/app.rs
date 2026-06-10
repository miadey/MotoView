//! The egui/eframe GUI shell. Deliberately THIN: every decision of substance
//! (which widget renders a node, how to parse diagnostics, file IO, spawning
//! the compiler) lives in `backend` / `highlight` and is unit-tested headless.
//!
//! Layout:
//!   * left  : a file list of the project's `.mview` files + project actions.
//!   * center: a code editor over the open `.mview` with `.mview` syntax
//!             highlighting + a diagnostics list (and inline file/line refs).
//!   * right : the PREVIEW panel — runs `motoview preview --json` and renders
//!             the returned IR forest as NATIVE egui widgets (the 4th renderer
//!             of the same IR, after HTML/SwiftUI/Compose).

use std::path::PathBuf;

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText};

use motokostudio::backend::{self, CommandReport, PreviewResult, Session, SessionEvent, UiNode, WidgetKind};
use motokostudio::highlight::{classify_line, TokenClass};

/// The whole application state.
pub struct StudioApp {
    project_dir: Option<PathBuf>,
    files: Vec<PathBuf>,
    open_file: Option<PathBuf>,
    editor_text: String,
    dirty: bool,

    diagnostics: Option<CommandReport>,
    preview: Option<PreviewResult>,
    route: String,
    status: String,

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
            files: Vec::new(),
            open_file: None,
            editor_text: String::new(),
            dirty: false,
            diagnostics: None,
            preview: None,
            route: String::new(),
            status: "Open a MotoView project (a folder with motoview.json) to begin.".to_string(),
            session: Vec::new(),
            session_route: None,
            replay_error: None,
        }
    }
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Slightly larger default text so code is readable.
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, fid) in style.text_styles.iter_mut() {
            fid.size *= 1.05;
        }
        cc.egui_ctx.set_style(style);

        let mut app = Self::default();
        // If launched from inside a project, auto-open it.
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("motoview.json").exists() {
                app.set_project(cwd);
            }
        }
        app
    }

    fn set_project(&mut self, dir: PathBuf) {
        self.files = backend::list_mview_files(&dir);
        self.status = format!("Opened {} — {} .mview file(s).", dir.display(), self.files.len());
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

    fn run_preview(&mut self) {
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
            // A fresh preview is the INITIAL render: clear the live session and
            // pin replays to whichever route we just rendered.
            self.session.clear();
            self.session_route = route.map(str::to_string);
            self.replay_error = None;
        }
    }

    // --- R17: LIVE preview via replay -------------------------------------

    /// Dispatch one event through the page's dispatch+render: append it to the
    /// accumulated session, replay the WHOLE session via `motoview preview
    /// --replay` (moc -r, no deploy), and swap in the resulting forest so
    /// page-local state visibly mutates. On replay failure the previous forest
    /// is kept and the error is surfaced in the diagnostics area.
    fn dispatch_event(&mut self, ev: SessionEvent) {
        let Some(dir) = self.project_dir.clone() else {
            return;
        };
        self.session.push(ev);
        let route = self.session_route.as_deref();
        match backend::replay_dispatch(&dir, route, &self.session) {
            Ok(forest) => {
                let n = forest.len();
                // Preserve the prior raw_json/ok wrapper shape via a fresh result.
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
                // Roll back the event that failed so the session stays in sync
                // with the forest still on screen.
                self.session.pop();
                self.replay_error = Some(e.clone());
                self.status = format!("Replay failed: {e}");
            }
        }
    }

    /// Reset the live session back to the initial render (re-run plain preview).
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
        self.bottom_bar(ctx);
        self.central_editor(ctx);
    }
}

impl StudioApp {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("MotokoStudio");
                ui.label(RichText::new("native · webview-free").weak());
                ui.separator();
                if ui.button("Open project…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.set_project(dir);
                    }
                }
                let has_project = self.project_dir.is_some();
                ui.add_enabled_ui(has_project, |ui| {
                    if ui.button("Check").clicked() {
                        self.run_check();
                    }
                    if ui.button("Lint").clicked() {
                        self.run_lint();
                    }
                    ui.separator();
                    ui.label("route:");
                    ui.add(egui::TextEdit::singleline(&mut self.route).hint_text("/").desired_width(120.0));
                    if ui.button("Preview").clicked() {
                        self.run_preview();
                    }
                });
                let dirty = self.dirty;
                ui.add_enabled_ui(self.open_file.is_some(), |ui| {
                    if ui.button(if dirty { "Save *" } else { "Save" }).clicked() {
                        self.save();
                    }
                });
            });
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("files").default_width(240.0).show(ctx, |ui| {
            ui.heading("Files");
            ui.separator();
            if self.files.is_empty() {
                ui.label(RichText::new("No .mview files.").weak());
            }
            let root = self.project_dir.clone();
            let mut to_open: Option<PathBuf> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for f in &self.files {
                    let label = display_name(&root, f);
                    let selected = self.open_file.as_ref() == Some(f);
                    if ui.selectable_label(selected, label).clicked() {
                        to_open = Some(f.clone());
                    }
                }
            });
            if let Some(p) = to_open {
                self.open(p);
            }
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        // Event dispatched by a button click this frame (applied after render so
        // we don't borrow `self` while egui borrows the forest).
        let mut clicked: Option<SessionEvent> = None;
        let mut do_reset = false;

        egui::SidePanel::right("preview").default_width(360.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Preview");
                ui.label(RichText::new("native egui · LIVE replay").weak());
            });
            // Live-session controls: a reset back to the initial render + a
            // count of how many events are in the accumulated session.
            ui.horizontal(|ui| {
                let has_preview = matches!(&self.preview, Some(p) if p.ok);
                ui.add_enabled_ui(has_preview && !self.session.is_empty(), |ui| {
                    if ui.button("Reset").clicked() {
                        do_reset = true;
                    }
                });
                if !self.session.is_empty() {
                    ui.label(
                        RichText::new(format!("{} event(s) dispatched", self.session.len()))
                            .weak(),
                    );
                } else if has_preview {
                    ui.label(RichText::new("click a button to dispatch").weak());
                }
            });
            ui.separator();
            if let Some(err) = &self.replay_error {
                ui.colored_label(Color32::LIGHT_RED, "Replay failed (forest unchanged):");
                ui.label(RichText::new(err).weak());
                ui.separator();
            }
            match &self.preview {
                None => {
                    ui.label("Press Preview to render the page IR forest as native widgets.");
                }
                Some(p) if !p.ok => {
                    ui.colored_label(Color32::LIGHT_RED, "Preview failed.");
                    if let Some(n) = &p.note {
                        ui.label(RichText::new(n).weak());
                    }
                }
                Some(p) => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if p.forest.is_empty() {
                            ui.label(RichText::new("(empty forest)").weak());
                        }
                        for node in &p.forest {
                            render_node(ui, node, &mut clicked);
                        }
                    });
                }
            }
        });

        // Apply any click AFTER the panel closure releases its borrow of `self`.
        if do_reset {
            self.reset_session();
        } else if let Some(ev) = clicked {
            self.dispatch_event(ev);
        }
    }

    fn bottom_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).monospace());
            });
        });
    }

    fn central_editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Header: open file name.
            ui.horizontal(|ui| match &self.open_file {
                Some(p) => {
                    ui.label(RichText::new(p.display().to_string()).strong());
                    if self.dirty {
                        ui.colored_label(Color32::YELLOW, "● unsaved");
                    }
                }
                None => {
                    ui.label(RichText::new("No file open — pick one on the left.").weak());
                }
            });
            ui.separator();

            // Diagnostics for the open file (and others), shown as a list with
            // file:line refs (this is the "inline as a list" path).
            if let Some(report) = &self.diagnostics {
                diagnostics_view(ui, report);
                ui.separator();
            }

            // The editor itself, with a `.mview` syntax-highlight layouter.
            let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                let mut job = highlight_job(text, ui.visuals().dark_mode);
                job.wrap.max_width = wrap_width;
                ui.fonts(|f| f.layout_job(job))
            };

            egui::ScrollArea::vertical().show(ui, |ui| {
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut self.editor_text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .layouter(&mut layouter),
                );
                if resp.changed() {
                    self.dirty = true;
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Diagnostics view
// ---------------------------------------------------------------------------

fn diagnostics_view(ui: &mut egui::Ui, report: &CommandReport) {
    let errs = report.diagnostics.iter().filter(|d| d.is_error()).count();
    let warns = report.diagnostics.iter().filter(|d| d.is_warning()).count();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Diagnostics").strong());
        ui.colored_label(Color32::LIGHT_RED, format!("{errs} error(s)"));
        ui.colored_label(Color32::from_rgb(220, 180, 60), format!("{warns} warning(s)"));
    });
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .id_salt("diags")
        .show(ui, |ui| {
            if report.diagnostics.is_empty() {
                ui.label(RichText::new("clean").weak());
            }
            for d in &report.diagnostics {
                let color = if d.is_error() {
                    Color32::LIGHT_RED
                } else if d.is_warning() {
                    Color32::from_rgb(220, 180, 60)
                } else {
                    Color32::GRAY
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(color, format!("[{}]", d.rule));
                    let loc = if d.line > 0 {
                        format!("{}:{}:{}", d.file, d.line, d.col)
                    } else {
                        d.file.clone()
                    };
                    if !loc.is_empty() {
                        ui.label(RichText::new(loc).monospace().weak());
                    }
                    ui.label(&d.message);
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Syntax highlighting layouter (uses the pure classifier)
// ---------------------------------------------------------------------------

fn class_color(class: TokenClass, dark: bool) -> Color32 {
    if dark {
        match class {
            TokenClass::Plain => Color32::from_rgb(220, 220, 220),
            TokenClass::Directive => Color32::from_rgb(197, 134, 192), // purple
            TokenClass::Tag => Color32::from_rgb(86, 156, 214),        // blue
            TokenClass::StringLit => Color32::from_rgb(206, 145, 120), // orange
            TokenClass::Comment => Color32::from_rgb(106, 153, 85),    // green
        }
    } else {
        match class {
            TokenClass::Plain => Color32::from_rgb(40, 40, 40),
            TokenClass::Directive => Color32::from_rgb(128, 0, 128),
            TokenClass::Tag => Color32::from_rgb(0, 0, 200),
            TokenClass::StringLit => Color32::from_rgb(160, 60, 0),
            TokenClass::Comment => Color32::from_rgb(0, 110, 0),
        }
    }
}

/// Build a `LayoutJob` for the whole document by classifying it line-by-line.
fn highlight_job(text: &str, dark: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let font = FontId::monospace(13.0);
    // Iterate lines while preserving the trailing `\n` so offsets stay sane.
    let mut rest = text;
    while !rest.is_empty() {
        let (line, tail, had_nl) = match rest.find('\n') {
            Some(p) => (&rest[..p], &rest[p + 1..], true),
            None => (rest, "", false),
        };
        for span in classify_line(line) {
            let fmt = TextFormat::simple(font.clone(), class_color(span.class, dark));
            job.append(&line[span.start..span.end], 0.0, fmt);
        }
        if had_nl {
            job.append("\n", 0.0, TextFormat::simple(font.clone(), class_color(TokenClass::Plain, dark)));
        }
        rest = tail;
    }
    job
}

// ---------------------------------------------------------------------------
// The NATIVE IR renderer (thin — decisions come from backend::widget_kind)
// ---------------------------------------------------------------------------

/// Render one IR node as native egui widgets. The widget CHOICE is the pure,
/// tested `widget_kind`; this fn just performs the chosen widget.
///
/// `clicked` is the LIVE-preview sink: when a clickable node (button/submit)
/// is pressed and `event_from_node` yields a [`SessionEvent`], we record it so
/// the caller can dispatch it through `--replay`. We record the FIRST click of
/// the frame; egui only reports one press per frame anyway.
fn render_node(ui: &mut egui::Ui, node: &UiNode, clicked: &mut Option<SessionEvent>) {
    match backend::widget_kind(node) {
        WidgetKind::Label => {
            if let UiNode::Text { value } = node {
                if !value.trim().is_empty() {
                    ui.label(value);
                }
            }
        }
        WidgetKind::RawLabel => {
            if let UiNode::Raw { html } = node {
                let txt = backend::strip_html(html);
                if !txt.trim().is_empty() {
                    ui.label(RichText::new(txt).italics().weak());
                }
            }
        }
        WidgetKind::Button => {
            let label = {
                let t = backend::node_text(node);
                if t.is_empty() {
                    "(button)".to_string()
                } else {
                    t
                }
            };
            // LIVE: a real egui Button. When pressed, extract its handler+args
            // (event_from_node) and queue the event for replay dispatch.
            if ui.button(label).clicked() {
                if clicked.is_none() {
                    if let Some(ev) = backend::event_from_node(node) {
                        *clicked = Some(ev);
                    }
                }
            }
        }
        WidgetKind::Input => {
            // Show a disabled placeholder text box reflecting the field.
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
            // Inline-ish wrappers: lay children in a horizontal, wrapping row.
            // Headings get a heading style.
            if let UiNode::El { tag, children, .. } = node {
                let is_heading = matches!(tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                ui.horizontal_wrapped(|ui| {
                    if is_heading {
                        let t = backend::node_text(node);
                        ui.heading(t);
                    } else {
                        for c in children {
                            render_node(ui, c, clicked);
                        }
                    }
                });
            }
        }
        WidgetKind::Group | WidgetKind::Unknown => {
            // Block container: a bordered vertical group holding its children.
            // A <form> with a submit handler also dispatches via replay when a
            // submit-type control inside it is pressed (the button case above),
            // so the group itself just lays out children.
            if let UiNode::El { children, .. } = node {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.vertical(|ui| {
                        for c in children {
                            render_node(ui, c, clicked);
                        }
                    });
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

fn display_name(root: &Option<PathBuf>, file: &PathBuf) -> String {
    if let Some(r) = root {
        if let Ok(rel) = file.strip_prefix(r) {
            return rel.display().to_string();
        }
    }
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}
