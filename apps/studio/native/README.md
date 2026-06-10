# MotokoStudio — native (webview-free)

A genuinely **webview-free**, fully-ours native desktop IDE for MotoView,
built on [`egui`/`eframe`](https://github.com/emilk/egui) (Rust, MIT).

This is the answer to "no Tauri, no WebView, no PWA shell." There is **no
embedded browser** anywhere in the binary. The only system dependencies are the
ones any native app has: the **linker** and the **GPU driver** (Metal/OpenGL on
macOS, via the Command-Line-Tools SDK). No full Xcode, no code-signing, no $99.

## What it does

- **File panel** — lists every `.mview` in the open project.
- **Code editor** — an egui `TextEdit` over the open `.mview`, with lightweight
  `.mview` **syntax highlighting** (directives `@page`/`@code`/…, tags, strings,
  comments) and a **Save** to disk.
- **Diagnostics** — runs the real `motoview check --json` / `lint --json` and
  shows the parsed `{severity, rule, message, file:line}` list (the unsecured
  `<form @submit>` → `secure-form` error shows up here, straight from the
  compiler).
- **Preview** — runs `motoview preview --json` and renders the returned **IR
  forest as NATIVE egui widgets**. This is the headline: the *fourth* renderer
  of the same MotoView UI-IR (after HTML, SwiftUI, Compose):
  - `el(div/section/ul/form/…)` → a bordered vertical group
  - `el(button)` / `<input type=submit>` → an egui `Button` (label = its text)
  - `el(span/p/h1…/a/…)` → an inline, wrapping row (headings get heading style)
  - `el(input/textarea/select)` → a (disabled, preview-only) text field
  - `text` → a label · `raw` → an italic label of the stripped/escaped HTML

## Run

```sh
cargo run --manifest-path apps/studio/native/Cargo.toml
```

It opens a real OS window, so it needs a **desktop GUI session** (it cannot run
headless). The compiler binary is resolved in this order:

1. `$MOTOVIEW` (explicit path),
2. `compiler/target/release/motoview` relative to the repo,
3. bare `motoview` on `$PATH`.

So from the repo root, with `compiler/target/release/motoview` built, it just
works. Then **Open project…** and point it at any folder containing a
`motoview.json` (e.g. `examples/counter`, or `apps/studio`).

## Backend choice: `glow`, not `wgpu`

`eframe` is configured `default-features=false, features=["glow",
"default_fonts"]`. The `glow` (OpenGL) backend is lighter than `wgpu` and links
cleanly with Command-Line-Tools only. On macOS it pulls in AppKit / QuartzCore /
Metal / OpenGL / CoreGraphics — **all present in the CLT macOS SDK** — and
crucially **no WebKit**. Verify it yourself:

```sh
cargo build --manifest-path apps/studio/native/Cargo.toml
otool -L apps/studio/native/target/debug/motokostudio | grep -i webkit   # → nothing
```

## Standalone crate (not in the compiler workspace)

This crate declares its own empty `[workspace]`, so it is **not** a member of
the `compiler/` workspace. That keeps egui/eframe's heavy dependency graph from
bloating compiler builds. Build/test it on its own manifest path.

## Tests (headless — no window needed)

```sh
cargo test --manifest-path apps/studio/native/Cargo.toml
```

All logic of substance is in pure, window-free modules:

- `backend.rs` — spawn `motoview`, parse `--json`, file IO, parse the IR forest
  (`parse_forest`), and the pure IR→widget decision (`widget_kind`).
  - **Backend tests** run the *real* `motoview` binary against
    `tests/fixtures/unsecured` (an unsecured `<form @submit>`) and assert the
    `secure-form` error comes back parsed. File list/read/write round-trip too.
  - **IR-mapping tests** parse a sample forest (button + text + raw + nested el)
    into the expected tree and check `widget_kind` for each node.
- `highlight.rs` — the pure `.mview` token classifier behind the editor's
  highlight layouter (also unit-tested).

The GUI (`app.rs`) is a thin shell over those tested functions.
