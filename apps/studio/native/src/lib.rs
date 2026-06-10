//! MotokoStudio (native) — library surface.
//!
//! The crate is split so that ALL non-GUI logic lives in modules that compile
//! and unit-test without egui/eframe or a window:
//!   * [`backend`] — spawn the `motoview` binary, parse `--json`, file ops,
//!     parse the IR forest, and the pure IR->widget decision (`widget_kind`).
//!   * [`highlight`] — a tiny, pure `.mview` token classifier used by the
//!     editor's syntax-highlight layouter (also unit-tested headless).
//!
//! The GUI (`app`) is only compiled into the binary; it is a thin shell over
//! these tested functions.

pub mod app;
pub mod backend;
pub mod highlight;
pub mod theme;

pub use backend::{
    expand_path, home_dir, list_mview_files, node_text, open_project, parse_forest, read_file,
    resolve_motoview, run_build, run_check, run_fmt, run_lint, run_preview, strip_html,
    widget_kind, write_file, CommandReport, Diagnostic, OpenOutcome, PreviewResult, UiNode,
    WidgetKind,
};
