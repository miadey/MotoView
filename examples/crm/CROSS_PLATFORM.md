# CRM — one source, three native targets, one UI-IR forest

This CRM is written **once**, in MotoView source under [`src/`](src/):

- [`src/Pages/Board.mview`](src/Pages/Board.mview) — the pipeline board (the
  `Pipeline` heading, the `+ New deal` button, the kanban columns, the deal
  cards and their move/remove buttons).
- [`src/Layouts/CrmLayout.mview`](src/Layouts/CrmLayout.mview) — the page shell.
- [`src/Services/Crm.mo`](src/Services/Crm.mo) — the stateful canister service.

`moc -r` drives the page's render to produce a single portable **UI-IR forest**
(JSON; nodes are `{"t":"el",…}` / `{"t":"text",…}` / `{"t":"raw",…}`). That one
forest is what every renderer consumes. There is **no per-platform UI code** —
the platforms differ only in how they map the SAME forest to native widgets.

```
                examples/crm/src/*.mview  (ONE source)
                              │
                  moc -r  ──► UI-IR forest (JSON)
                              │
        ┌─────────────────────┼──────────────────────┐
        ▼                     ▼                      ▼
   WEB (canister)       DESKTOP (egui)          iOS (SwiftUI)
   HTML render          native widgets          native views
```

## What is proven, and how

### WEB — Playwright on the live canister
The deployed CRM canister was driven end-to-end with Playwright (real clicks:
add a deal, move it across columns, remove it). That covered web *interactivity*
against the live service. See the `e2e/` suite in this folder.

### The committed forest fixture (the shared contract)
`motoview preview examples/crm` emits the Board page's UI-IR forest as JSON. That
exact output is committed as a fixture in **both** native trees, so the desktop
and iOS tests feed on the identical bytes that the web build renders from:

- `apps/studio/native/tests/fixtures/crm.forest.json`
- `clients/ios/Tests/Fixtures/crm.forest.json`

Regenerate (the two files are byte-identical):

```
compiler/target/release/motoview preview examples/crm | grep '^\[' \
  > apps/studio/native/tests/fixtures/crm.forest.json
cp apps/studio/native/tests/fixtures/crm.forest.json \
   clients/ios/Tests/Fixtures/crm.forest.json
```

### DESKTOP — egui (Rust / eframe)
`apps/studio/native` parses the committed forest with the SAME `backend::parse_forest`
the Studio preview panel uses, and asserts `backend::widget_kind` (the pure
decision `app::render_node` performs) maps the CRM nodes to the right egui
widgets:

- `+ New deal` and every per-card button → `WidgetKind::Button`
- kanban columns / card containers (`<section>`/`<article>`/`<div>`) → `WidgetKind::Group`
- deal titles and the `Pipeline` heading → `WidgetKind::Inline` (egui headings/labels)

Run:

```
cargo test --manifest-path apps/studio/native/Cargo.toml
```

(tests: `crm_fixture_is_the_crm_board`, `crm_forest_maps_to_egui_widgets`.)

### iOS — SwiftUI (Swift)
`clients/ios` parses the SAME committed forest through the **real Rust core over
the C ABI** (`MotoViewCore.parseForest`, the FFI), then asserts the SwiftUI
mapping that `NativeView` performs, via the pure `nativeWidgetKind` shadow of its
dispatch:

- `+ New deal` and every per-card button → `Button`
- kanban columns / card containers → `VStack`
- deal titles and the `Pipeline` heading → `Text`

Run (host executable — the same assertions XCTest runs on a full-Xcode machine):

```
cd clients/ios && swift build && swift run motoview-smoke
```

(step `[4/4]`; XCTest mirror: `testCRMForestIsTheCRMBoard`,
`testCRMForestMapsToSwiftUIWidgets`.)

## Honest scope

These native checks prove the **forest → native-widget mapping** that each
renderer uses — they do **not** launch the egui window or an iOS simulator/device
(headless CI; full Xcode is not present on the build machine). The mapping is the
load-bearing decision (`render_node`/`NativeView` are thin performers over it), so
proving the mapping proves what those renderers would draw. Web *interactivity*
was proven separately with Playwright against the live canister.
