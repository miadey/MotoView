# __NAME__

A MotoView app scaffolded from the **secure-form** template — a
secure-by-construction `<form>` pattern.

## What it demonstrates

`src/Pages/Home.mview` is a contact form that is safe by construction:

- **`@authorize`** gates the page to an authenticated caller (add
  `role="Admin"` to also require a role).
- **`<form @submit="submit" secure>`** makes the runtime mint an HMAC-SHA256
  token binding path, handler, caller, nonce, expiry and field-schema; the
  server re-derives the MAC and rejects replays / tampering. Without `secure`,
  a state-mutating form is a hard lint + build **error** (deny-by-default).
- **`validate { ... }`** runs INSIDE the handler, server-side — one validation
  path a crafted client cannot skip.

## Run it

```bash
motoview build        # compile .mview -> .mvbuild/main.mo
motoview lint         # security lint — this template is clean (0 errors)
motoview dev          # build, then `dfx deploy` to a local replica
```

> The generated actor lives in `.mvbuild/main.mo` — a build artifact, gitignored
> and regenerated on every build. Edit the `.mview` files in `src/`.
