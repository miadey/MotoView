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

## Backend choice: `wgpu` (Metal), not `glow`

`eframe` is configured `default-features=false, features=["wgpu",
"default_fonts"]`. The `wgpu` backend targets **Metal** on macOS; we prefer it
over the legacy `glow` (OpenGL) backend because Apple has **deprecated OpenGL**,
so glow is a long-term risk. Metal is a system GPU framework present in the
Command-Line-Tools macOS SDK (`/System/Library/Frameworks/Metal.framework`), so
there is still **no full Xcode, no code-signing, no $99, and no WebKit** — wgpu
just pulls a heavier dep graph (naga + the wgpu/wgpu-hal/metal stack) and yields
a larger binary than glow would. Verify there is no embedded browser yourself:

```sh
cargo build --manifest-path apps/studio/native/Cargo.toml
otool -L apps/studio/native/target/debug/motokostudio | grep -i webkit   # → nothing
```

## Packaging: `.app` + `.dmg` (`bundle.sh`)

```sh
bash apps/studio/native/bundle.sh
#   -> apps/studio/native/dist/MotokoStudio.app   (a real app bundle)
#   -> apps/studio/native/dist/MotokoStudio.dmg   (a distributable disk image)
```

`bundle.sh` is our **own** dependency-free packager — no `cargo-bundle`, no
third-party bundler, only stock Command-Line-Tools (`cargo`, `codesign`,
`hdiutil`, `plutil`, `file`, `lipo`, `iconutil`). It:

1. **gen-icon** — builds the standalone [`icongen/`](icongen/) crate and runs it
   to render [`assets/icon.svg`](assets/icon.svg) into an `AppIcon.iconset/`,
   then `iconutil -c icns` folds that into `AppIcon.icns` (see **Icon** below),
2. **universal build** — `cargo build --release --target aarch64-apple-darwin`
   **and** `--target x86_64-apple-darwin`, then `lipo -create` welds them into
   one fat binary that runs on Apple Silicon **and** Intel (see **Universal**),
3. assembles `dist/MotokoStudio.app/Contents/{Info.plist, MacOS/motokostudio,
   Resources/AppIcon.icns, PkgInfo}` (the [`Info.plist`](Info.plist) template,
   with the version stamped from `Cargo.toml` and `CFBundleIconFile=AppIcon`;
   `plutil -lint` must pass),
4. **ad-hoc** code-signs it (`codesign --sign -`), and
5. `hdiutil create … -format UDZO` makes the `.dmg`.

The `.app`, `.dmg`, generated `AppIcon.iconset/` and `AppIcon.icns` are all
**build outputs** — they live under `dist/`, which is `.gitignore`d. Only the
*script*, the *plist template*, the *icon source* `assets/icon.svg`, and the
*`icongen/` crate source* are committed; rebuild the artifacts any time with
`bash bundle.sh`.

### Icon — pure-Rust SVG → `.icns` (no system renderer)

The app icon is the **MotoView brand mark** (purple rounded square, white "M",
white dot), kept in [`assets/icon.svg`](assets/icon.svg) at a 1024-unit viewBox.
This box has **no system SVG renderer** (`rsvg`/`resvg`/`inkscape` are absent),
so [`icongen/`](icongen/) — a **separate** standalone crate (its own empty
`[workspace]`, **not** a dependency of the studio binary) — rasterizes it in
**100% Rust** via `usvg` + `resvg` + `tiny-skia`. It renders the ten macOS
iconset PNGs (16/32/128/256/512 at @1x and @2x, i.e. up to 1024px) **directly at
each target pixel size** from the vector source, so every size is crisp. The
heavy SVG-renderer dependency graph therefore **never** enters the shipping
studio binary — `icongen` is a build-time tool that `bundle.sh` builds, runs,
and discards.

The result is the brand **mark** (a programmatic geometric logo), not bespoke
artwork. Regenerate it standalone:

```sh
cargo build --release --manifest-path apps/studio/native/icongen/Cargo.toml
apps/studio/native/icongen/target/release/icongen \
    apps/studio/native/assets/icon.svg /tmp/AppIcon.iconset
iconutil -c icns -o /tmp/AppIcon.icns /tmp/AppIcon.iconset
file /tmp/AppIcon.icns          # -> Mac OS X icon
```

### Universal binary (arm64 + x86_64)

By **default** `bundle.sh` produces a **universal** binary: it builds the
release crate for **both** `aarch64-apple-darwin` and `x86_64-apple-darwin`
(both `rustup` targets must be installed) and `lipo -create`s them into one fat
Mach-O, so the same `.app`/`.dmg` runs on Apple Silicon **and** Intel Macs.
`codesign` then signs the universal bundle; verify it with:

```sh
lipo -info dist/MotokoStudio.app/Contents/MacOS/motokostudio   # -> arm64 x86_64
codesign -dvv dist/MotokoStudio.app 2>&1 | grep Format         # -> ... universal (x86_64 arm64)
```

Opt out to host-arch-only (skip the cross-build) with `--arch host`:

```sh
bash apps/studio/native/bundle.sh --arch host   # -> a single-arch (host) binary
```

If the `x86_64` cross-build genuinely fails (e.g. a dep that won't
cross-compile), `bundle.sh` does **not** fake a universal binary — it falls back
to the host arch alone, prints a loud `WARNING`, and reports the real arch in
its summary. (On this toolchain the cross-build succeeds, so the default really
is universal.)

### Signing caveat (honest)

The default is an **ad-hoc** signature: no Apple account, no certificate. That
is *required* on Apple Silicon — an unsigned bundle is killed as "damaged" — and
it lets the app launch on **this** Mac, and on others via right-click → **Open**
(a one-time Gatekeeper override). A plain double-click on **another** Mac will
still warn.

To distribute **without** any warning on any Mac you must **notarize**, and that
is the one Apple toll: it needs a paid **Apple Developer ID** certificate
($99/yr) plus Xcode's `notarytool`. That path is supported but not faked:

```sh
bash apps/studio/native/bundle.sh \
     --sign "Developer ID Application: Your Name (TEAMID)" \
     --notarize-profile <your-notarytool-keychain-profile>
```

With a real `--sign` identity the script code-signs with the hardened runtime +
trusted timestamp and signs the `.dmg`; with `--notarize-profile` it also runs
`notarytool submit --wait` and `stapler staple`. Neither a cert nor `notarytool`
ships here, so the default ad-hoc path is what runs out of the box.

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
