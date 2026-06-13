# Self-hosted font

`roboto.woff2` (served by the canister at `/fonts/roboto.woff2`, base64-embedded
in `runtime/src/ClientAssets.mo`) is **Roboto** — a variable font, weights
400–700, Latin subset (≈43 KB).

- **Font:** Roboto by Christian Robertson / Google
- **License:** Apache License 2.0 — https://www.apache.org/licenses/LICENSE-2.0
- **Source:** Google Fonts (https://fonts.google.com/specimen/Roboto)

Apache-2.0 permits redistribution; this file is the required attribution/notice.
It is loaded lazily, only when a Material theme (`data-theme="material-*"`) is
active (those themes set `font-family: Roboto`).
