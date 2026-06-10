# MotokoStudio — AI generation contract (OFF-canister, design-time)

This is the **interface** for the AI generation step of the MotokoStudio loop. It
is a documented stub, **not** a faked LLM. There is **no on-canister model**: the
LLM call is an external, off-canister, design-time HTTP request. The studio
canister never invokes a model; it only holds session state (see
`apps/studio/src/Services/Studio.mo`).

The contract has one job: turn a prompt + the project's real backend signatures
into **one complete, type-checked `.mview` artifact** — and that artifact is only
accepted once the compiler gate (`tools/studio/validate.sh`) passes.

---

## Why this is safe even though an LLM writes the code

Security-by-construction does **not** depend on the model behaving. It depends on
the **unbypassable compiler**:

- The model can hallucinate an insecure form, a wrong type, a missing `secure`
  attribute. It does not matter.
- The output is fed to `tools/studio/validate.sh`, which runs `motoview lint` and
  `motoview check`. A state-mutating `<form @submit>` without `secure` is a hard
  build **Error** (Slice 1, deny-by-default). A type error fails `check`.
- If either fails, the gate **exits nonzero and the artifact is refused** — it is
  never written to the project, never deployed, never previewed.

So the LLM is an untrusted suggestion engine. The compiler is the authority.

---

## Input

```jsonc
{
  // The human's design request.
  "prompt": "A page to create a forum topic: title, category, body. Signed-in only.",

  // The BINDING PALETTE: the project's REAL Motoko service signatures, extracted
  // from src/Services/*.mo. This is what keeps generation "always bound to the
  // backend" — the model may only call functions that actually exist with the
  // types they actually have. Extract these from the .mo files (their public
  // class methods), e.g. via `motoview check` having compiled the actor, or by
  // parsing the `public func` signatures.
  "services": [
    {
      "name": "Forum",
      "methods": [
        "categories() : [Forum.Category]",
        "createTopic(caller : Principal, handle : Text, categoryId : Nat, title : Text, tags : [Text], body : Text) : Nat",
        "categoryName(id : Nat) : Text"
      ],
      "types": [
        "Category = { id : Nat; name : Text; color : Text }"
      ]
    },
    {
      "name": "Identity",
      "methods": [
        "isBound(p : Principal) : Bool",
        "handleOf(p : Principal) : Text"
      ]
    }
  ],

  // The runtime security modules available to wire to (advisory; the model is
  // told to prefer secure patterns). Mirrors runtime/src/*.mo.
  "securityModules": ["Security", "Roles", "EncStore", "VetKeys", "Audit", "ChainKey", "WalletAuth", "CertV2"],

  // OPTIONAL: the current source, for an "edit this" request rather than "create".
  "currentSource": null
}
```

## Output

Exactly **one** complete `.mview` artifact as text — template + `@code` +
typed state — and nothing else:

```
@page "/forum/new"
@layout ForumLayout
@title "New topic"

<form @submit="create" secure>
  <ValidationSummary />
  <InputText name="title" bind="@title" required />
  <TextArea  name="body"  bind="@body"  required />
  <Button kind="primary" type="submit">Create</Button>
</form>

@code {
  var title : Text = "";
  var body  : Text = "";
  func create(ctx : Context) : async () {
    validate {
      title required "Give your topic a title.";
      body  required "Write something.";
    };
    let handle = Identity.handleOf(ctx.caller);
    ignore Forum.createTopic(ctx.caller, handle, 1, title, [], body);
    title := ""; body := "";
    toast("Topic created!");
  };
}
```

### Hard requirements the model is instructed to follow

These mirror what the gate enforces, so generations land on the right side of it:

1. **No frontend JavaScript.** All logic is Motoko in `@code`. Events POST to the
   canister; the WASM client + glue are not the model's concern.
2. **Every state-mutating `<form @submit>` MUST be `secure`.** (Otherwise the gate
   refuses it.) Non-mutating forms (search/filter that only read) may omit it.
3. **Only call service functions that exist** in the binding palette, with the
   exact types given. No invented endpoints.
4. **Validate server-side** in the handler with a `validate { ... }` block.
5. **Gate sensitive pages** with `@authorize` (and `@authorize role="..."` where a
   role is required).
6. **Never pass user input to `@raw`** — it bypasses HTML escaping.
7. **Wallet spends** must call `ctx.authorizeSpend(...)` and get `true` BEFORE any
   `ChainKey` signing (intent-bound, single-use, velocity-limited).

## The off-canister call (NOT implemented here — this is the seam)

The generation endpoint is intentionally **left as an interface**. A real
implementation would:

```
POST  $MOTOVIEW_STUDIO_LLM_ENDPOINT      # external; e.g. an Anthropic/OpenAI gateway
Authorization: Bearer $MOTOVIEW_STUDIO_LLM_KEY
Content-Type: application/json
Body: { system: <the rules above>, input: <the Input JSON> }
->    { mview: "<the generated .mview text>" }
```

This file does **not** ship a faked model and does **not** embed a key. Wiring a
real endpoint is a deployment choice; the **only** thing the studio guarantees is
that whatever comes back must pass `tools/studio/validate.sh` before it can be
saved.

## The mandatory gate (always, regardless of model)

```sh
# 1. write the model's output to a scratch project
cp generated.mview <project>/src/Pages/<Name>.mview

# 2. the artifact is ONLY saveable if BOTH gates pass:
tools/studio/validate.sh <project>   # exits nonzero -> discard the generation

# 3. on success, it can be deployed + previewed (see README.md)
```

If step 2 fails, the generation is discarded and the user is shown the compiler's
error. The model is asked to try again with that error as feedback. The artifact
is **never** saved in a failing state.
