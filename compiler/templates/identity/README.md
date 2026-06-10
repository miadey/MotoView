# __NAME__

A MotoView app scaffolded from the **identity** template — Internet Identity
sign-in plus a role-gated action.

## What it demonstrates

`src/Pages/Home.mview` is an identity page that:

- Renders the **signed-in state server-side** from `ctx.caller` /
  `ctx.isAuthenticated` — never trusting the client for identity.
- Uses the hand-rolled **Internet Identity** login glue
  (`data-mv-signin`, no agent-js) to establish the session.
- Reads and writes **roles** through the runtime's persisted per-principal
  role store (`ctx.hasRole` / `ctx.callerRoles` / `ctx.claimRole`).
- Offers a **first-come Admin bootstrap**: `claimRole("Admin")` grants only if
  no principal holds it yet — a safe one-time seat claim, not an escalation.
- Puts the role-changing action behind a **`secure` form** (mutating → must be
  secure).

## Run it

```bash
motoview build        # compile .mview -> .mvbuild/main.mo
motoview lint         # security lint — this template is clean (0 errors)
motoview dev          # build, then `dfx deploy` to a local replica
```

> The generated actor lives in `.mvbuild/main.mo` — a build artifact, gitignored
> and regenerated on every build. Edit the `.mview` files in `src/`.
