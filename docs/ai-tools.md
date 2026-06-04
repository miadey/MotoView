---
title: Using AI (Claude, Copilot, Cursor)
section: Tooling
slug: ai-tools
---

# Using AI (Claude, Copilot, Cursor)

MotoView is a small framework, but it is an unusual one. You write `.mview` files, events run server-side, and there is no application JavaScript anywhere. General-purpose AI assistants do not know this. Left to their own training data, they will reach for React, sprinkle in `fetch`, invent directives, and reconstruct validation in the browser — exactly the patterns MotoView exists to remove.

To fix that, MotoView ships **assistant rule files** in every scaffolded project. They teach Claude, GitHub Copilot, and Cursor the real shape of the framework so their suggestions match the facts instead of guessing.

## Where the rule files live

`motoview new` writes three files, one per assistant, all sourced from the same canonical reference:

```text
skills/motoview/SKILL.md          # Claude / Claude Code skill
.github/copilot-instructions.md   # GitHub Copilot
.cursor/rules/motoview.mdc        # Cursor
```

They cover the same ground in each tool's expected format: the `.mview` file model, the directive list, the event flow, secure forms, the built-in components, and the protocol. Keeping them next to your code means they travel with the repository — clone it on another machine and the guidance comes along.

## How to use them

In most cases you do nothing. Each assistant picks its file up automatically.

- **Claude / Claude Code** — discovers `skills/motoview/SKILL.md` and loads it as a skill when you work in a MotoView project.
- **GitHub Copilot** — reads `.github/copilot-instructions.md` and applies it to completions and Copilot Chat across the workspace.
- **Cursor** — applies `.cursor/rules/motoview.mdc` as a project rule for inline edits and chat.

If your assistant supports manual context, point it at the relevant file directly, for example "follow `skills/motoview/SKILL.md`". When you upgrade the compiler, re-run the scaffold (or copy the latest files from a fresh `motoview new`) so the rules track the current directive set.

## What the rules prevent

The single most useful thing these files do is keep an assistant from "helpfully" adding a frontend. With the rules loaded, you get suggestions like this:

```razor
@page "/counter"

@code {
  var count : Nat = 0;
  func increment() { count += 1; };
}

<p>Count: @count</p>
<Button kind="primary" @click="increment">Increment</Button>
```

instead of a `useState` hook and an `onClick` that POSTs JSON. Handler arguments are evaluated server-side, so the assistant learns to write `@click="remove(item.id)"` rather than wiring up a client listener. For the full event flow see [Events](events.md); for signed forms and `validate`, see [Forms](forms.md) and [Validation](validation.md).

## Directive cheat-sheet

This is the table the rule files lean on. Keep it nearby — it is also the fastest way to sanity-check an AI suggestion.

```text
Page & metadata
  @page "/path"            route ("/products/{id}", typed "/orders/{id:Nat}")
  @layout NAME             choose a layout
  @title / @description    head metadata
  @canonical / @meta       canonical URL / extra meta tags
  @head                    inject into <head>

Structure
  @code { ...Motoko... }   state and handlers
  @style { ...css... }     scoped styles
  @theme { tokens }        design tokens
  @section "name" { }      named content for a layout
  @yield                   render the page body in a layout
  @slot "name"             component slot

Control flow
  @if EXPR { } else { }    conditional
  @for X in EXPR { }       loop
  @switch EXPR {           variant match
    case #Variant { } }

Output & effects
  @count  @user.name  @(expr)   inline output
  @effect Focus("#x")           also ScrollTo, Toast
  @animate                      transition hint

Events
  @click="save"            @click="save(arg)"
  @submit="send"           @input="onType"   @change

Access control
  @authorize               @authorize role="Admin"
```

For secure forms, the rules also remind the assistant to add the `secure` attribute and `bind="@model.field"`:

```razor
<form @submit="send" secure>
  <InputText name="name" label="Name" bind="@model.name" required />
  <ValidationSummary />
  <Button kind="primary">Save</Button>
</form>
```

```motoko
func send() {
  validate model {
    name required "Name is required";
  };
  // persist...
};
```

## A note on honesty

The rule files describe what is built and verified, and they mark planned work — keyed-region DOM patches, full Internet Identity login, certified query rendering — as Roadmap. If an assistant suggests one of those as if it exists today, that is a hallucination, not a feature. Trust the cheat-sheet and the rules over the model's instincts.
