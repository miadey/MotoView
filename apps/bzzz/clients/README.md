# Bzzz clients — web, desktop, mobile

Bzzz is a **server-driven** app: the canister renders the whole UI over HTTP.
So all three platforms run the **same** UI from one source of truth — there is
no duplicated frontend. Each client is a thin shell around the canister URL.

Set the canister URL once per client:
- Local replica: `http://<canister-id>.localhost:4955`
- Mainnet: `https://<canister-id>.icp0.io`

Replace `YOUR_CANISTER_ID` in the configs below with your deployed id
(`dfx canister id bzzz`).

---

## 1. Web / PWA  — nothing to build, served by the canister

The canister itself serves an installable PWA:
- `/manifest.webmanifest` and `/sw.js` are served by the MotoView runtime
  (`runtime/src/App.mo`), and `AppLayout.mview` links the manifest + registers
  the service worker.
- Open the canister URL in Chrome/Edge/Android → **Install app** / **Add to
  Home Screen**. The app is responsive (mobile breakpoints via `@media`) and the
  service worker caches the framework assets for fast loads + offline shell.

Verify locally after `dfx deploy`:
```bash
curl -s http://<cid>.localhost:4955/manifest.webmanifest
curl -s http://<cid>.localhost:4955/sw.js
```

## 2. Desktop — Tauri  (`desktop-tauri/`)

A Tauri v2 app whose main window (`src-tauri/tauri.conf.json` →
`app.windows[0].url`) loads the canister URL directly.

```bash
cd desktop-tauri
# one-time: install the Tauri CLI
cargo install tauri-cli --version '^2'
# edit src-tauri/tauri.conf.json -> set windows[0].url to your canister URL
cargo tauri build          # produces a native .app/.dmg/.exe/.AppImage
# or for a dev window:
cargo tauri dev
```
Add real icons under `src-tauri/icons/` (`cargo tauri icon path/to/logo.png`
generates every size) before bundling.

## 3. Mobile — Capacitor  (`mobile-capacitor/`)

A Capacitor app using the hosted-app pattern: `capacitor.config.json` →
`server.url` points the native iOS/Android webview at the canister.

```bash
cd mobile-capacitor
npm install
# edit capacitor.config.json -> set server.url to your canister URL
npm run add:ios        # or: npm run add:android   (needs Xcode / Android Studio)
npm run sync
npm run open:ios       # opens Xcode  (or open:android for Android Studio)
# then Build/Run from the IDE onto a simulator/device
```

---

## Status (honest)

- **Web/PWA**: fully working — the canister serves the app, the manifest, and the
  service worker; verified by `curl` after deploy. Installable on
  Chrome/Edge/Android; iOS via Add-to-Home-Screen.
- **Desktop (Tauri)** and **Mobile (Capacitor)**: real, correct, buildable
  project configs that load the canister URL. They were **not** compiled to
  binaries in this environment (Tauri needs the platform webview toolchain;
  Capacitor needs Xcode/Android Studio). Run the commands above on a machine
  with those toolchains to produce installable apps.
