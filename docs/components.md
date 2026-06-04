---
title: Components
section: The .mview Format
slug: components
---

# Components

Components let you carve a page into small, reusable pieces of markup and logic. In MotoView, a component is just another `.mview` file — same template syntax, same `@code` block, same Motoko underneath. If you can write a page, you can write a component.

In your templates, **any capitalized tag is a component**. A lowercase tag like `<div>` is plain HTML; a capitalized tag like `<Card>` resolves to a component.

## Authoring a component

App components live in `src/Components/*.mview`. The file name is the tag name. A component called `Greeting` lives at `src/Components/Greeting.mview`:

```razor
@code {
  param name : Text;
}

<p>Hello, @name!</p>
```

That's the whole contract: declare what comes in with `param`, then render markup that uses it.

## Declaring params

A component receives data through `param` declarations inside its `@code` block. Each `param` names a typed Motoko value the parent must — or may — provide:

```razor
@code {
  param title : Text;
  param count : Nat;
}

<section>
  <h2>@title</h2>
  <p>@count items</p>
</section>
```

### Defaults

Give a `param` a default with `=`. Params with a default are optional; the parent can omit them.

```razor
@code {
  param label : Text = "Save";
  param disabled : Bool = false;
}

<button disabled=@disabled>@label</button>
```

Because params are typed Motoko values, the compiler checks them. Pass the wrong type and the build fails — you find out at `motoview build`, not in the browser.

## Passing props

Set props as attributes on the capitalized tag. The attribute name matches the `param` name:

```razor
<Greeting name="Ada" />

<section title="Inbox" count=@unread />
```

String literals go in quotes; to pass a Motoko expression, use the `@` prefix or `@(...)`:

```razor
<Greeting name=@user.firstName />
<section title="Orders" count=@(orders.size()) />
```

Handler arguments and event wiring follow the same rules everywhere — see [Events](events.md) for how `@click` and friends behave inside components.

## Default content with @yield

A component can wrap content the parent puts between its tags. Render that content with `@yield`:

```razor
@code {
  param title : Text;
}

<article class="card">
  <h3>@title</h3>
  <div class="card-body">
    @yield
  </div>
</article>
```

Now the parent nests freely:

```razor
<Panel title="Profile">
  <p>Signed in as @user.name</p>
  <Button kind="primary">Edit</Button>
</Panel>
```

Everything between `<Panel>` and `</Panel>` lands where `@yield` sits.

## Named slots with @slot

When a component has more than one region to fill — say a header and a footer — declare named slots. Inside the component, mark the regions with `@slot "name"`:

```razor
<div class="dialog">
  <header>@slot "header"</header>
  <div class="dialog-body">@yield</div>
  <footer>@slot "footer"</footer>
</div>
```

The parent targets each slot with `@section`:

```razor
<Dialog>
  @section "header" {
    <h2>Confirm delete</h2>
  }

  <p>This action cannot be undone.</p>

  @section "footer" {
    <Button kind="danger">Delete</Button>
    <Button>Cancel</Button>
  }
</Dialog>
```

Content not wrapped in a `@section` flows into the default `@yield`. The same `@section` / `@yield` mechanism powers layouts — see [Layouts](layouts.md).

## Built-in semantic components

MotoView ships a set of semantic components so you reach for meaning, not utility-class soup. Write `<Button kind="primary">Save</Button>`, not a wall of classes.

- **`Button`** — `kind`, `size`
- **`Card`** — `title`
- **`Alert`** — `type`
- **`Badge`** — `type`
- **`InputText`**, **`InputEmail`**, **`InputNumber`**, **`TextArea`** — `name`, `label`, `bind`, `required`, `minLength`
- **`ValidationSummary`**
- **`Table`**
- **`PageHeader`**
- **`Grid`** — `columns`

A small example:

```razor
<Card title="New product">
  <InputText name="name" label="Name" bind="@model.name" required />
  <InputNumber name="price" label="Price" bind="@model.price" />
  <ValidationSummary />
  <Button kind="primary" size="lg">Create</Button>
</Card>
```

The form-oriented inputs pair with `bind=` and handler-side validation — see [Forms & Validation](forms.md) for the full flow.

## Where to go next

- [Events](events.md) — wiring `@click`, `@submit`, and friends.
- [Layouts](layouts.md) — `@section` / `@yield` at the page level.
- [Forms & Validation](forms.md) — secure forms and the built-in inputs.
