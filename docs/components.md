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

## Nesting and any param name

Components nest arbitrarily — a component can use another, which uses another — and both **props and `@children` thread through every level**, because each component compiles to its own render function (nesting is just function calls). You can also name a `param` anything: if the name collides with a Motoko keyword (`label`, `type`, `class`, …) the compiler **auto-mangles** it, so it just works.

```razor
@* Inner.mview *@   param label : Text
                    <span>@label</span>
@* Card.mview *@    <div><Inner label="@title" /> @children</div>
@* page *@          <Card><Inner label="hi" /></Card>   @* nests fine *@
```

## Built-in components

MotoView ships ~50 server-rendered, **CSS-only** built-ins (no JavaScript) — an authentic port of the Microsoft **Fluent UI 2** design system. Reach for meaning, not utility-class soup: `<Button appearance="primary">Save</Button>`. See them all live, with copy-paste snippets, in the **[/components gallery](https://github.com/miadey/MotoView)** (it's itself a MotoView page). Theme them with [`@theme` / `<ThemePicker/>`](styling-and-themes.md).

> Coverage note: these match each Fluent component's **look + main variants**, not the full Fluent React prop API. The exact props for any one are its `gen_builtin` arm in `compiler/src/codegen.rs`.

**Buttons & actions** — `Button` (`appearance` primary/secondary/outline/subtle/transparent/danger, `size`, `shape` rounded/circular/square, `icon`, `iconPosition`, `disabled`, `type`), `CompoundButton` (`appearance`, `icon`, `secondary`), `ToggleButton` (`appearance`, `name`, `checked`), `SplitButton`, `MenuButton`, `Menu` + `MenuItem` / `MenuItemCheckbox` / `MenuItemRadio`.

**Inputs & fields** — `Input` (`type`, `appearance`, `placeholder`, `size`), `InputText` / `InputEmail` / `InputNumber` / `TextArea` (`name`, `label`, `bind`, `required`, `minLength`), `Field` (`label`, `required`, `hint`, `validationState`), `Label`, `InfoLabel` (`label`, `info`), `SpinButton` (`min`/`max`/`step`), `Slider`, `Select`, `Combobox`, `Searchbox`, `Switch`, `Checkbox`, `Radio`, `ValidationSummary`.

**Data display** — `Badge` (`appearance` filled/ghost/outline/tint, `shape`, `size`, `color`), `CounterBadge` (`count`, `color`, `dot`), `PresenceBadge` (`status`), `Tag` (`appearance`, `size`, `shape`, `dismissible`), `TagGroup`, `InteractionTag`, `Avatar` (`name`, `size`, `shape`, `presence`, `color`, `active`, `badge`), `AvatarGroup`, `Persona`, `Rating`, `Image`, `ProgressBar` (`value`, `color`, `thickness`), `Spinner` (`size`, `label`), `Skeleton`, `Table`.

**Typography** — `Title1` / `Title2` / `Title3`, `Subtitle1` / `Subtitle2`, `Body1` / `Body2`, `Caption1` / `Caption2`, `Display`, and `Text` (`variant`) — the Fluent type ramp as components.

**Layout & cards** — `Card` (`title`) + `CardHeader` / `CardPreview` / `CardFooter`, `Grid` (`columns`), `Divider` (`vertical`, `appearance`), `PageHeader`, `Toolbar`, `Carousel`.

**Navigation & disclosure** — `Nav` + `NavItem` (`match`), `AppBar`, `Breadcrumb` + `BreadcrumbItem`, `TabList` + `Tab` (`match`), `Accordion` + `AccordionItem`, `Tree` + `TreeItem`.

**Feedback & overlays** (all CSS-only) — `MessageBar` / `Alert` (`intent`/`type`), `Tooltip`, `Popover`, `Dialog`, `Drawer`.

**Theme** — `<ThemeToggle/>` (light/dark) and `<ThemePicker/>` (Web Light/Dark, Teams Light/Dark, High Contrast). See [Styling & Themes](styling-and-themes.md).

A small example:

```razor
<Card title="New product">
  <Field label="Name" required>
    <InputText name="name" bind="@model.name" required />
  </Field>
  <Slider name="qty" min="1" max="10" value="3" />
  <ValidationSummary />
  <Button appearance="primary" icon="💾">Create</Button>
</Card>
```

The form-oriented inputs pair with `bind=` and handler-side validation — see [Forms & Validation](forms.md) for the full flow.

## Where to go next

- [Events](events.md) — wiring `@click`, `@submit`, and friends.
- [Layouts](layouts.md) — `@section` / `@yield` at the page level.
- [Forms & Validation](forms.md) — secure forms and the built-in inputs.
