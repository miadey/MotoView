---
title: Control Flow
section: The .mview Format
slug: control-flow
---

# Control Flow

MotoView templates are Motoko-aware. Inside a `.mview` file you can branch, loop, and
match over your data using a small set of `@`-directives that read like the markup they
produce. Everything runs server-side during render, so the values you reach for are just
plain Motoko expressions and variables declared in your [@code block](code-block.md).

There is no client-side templating: the server evaluates the control flow, produces HTML,
and the WASM client swaps it in. That means you can branch on a `caller`, a query result,
or anything else available to your canister without shipping a line of application
JavaScript.

## Inline output: `@expr` and `@(expr)`

The simplest form of control flow is printing a value. Reference a variable or a dotted
path directly with `@`, or wrap a larger expression in `@( ... )`.

```razor
@code {
  let user = { name = "Ada"; visits = 7 };
  var count : Nat = 3;
}

<p>Welcome back, @user.name.</p>
<p>You have visited @count times.</p>
<p>Next milestone: @(count + 1)</p>
```

Use the bare form (`@count`, `@user.name`) for identifiers and simple member access. Reach
for the parenthesized form (`@(count + 1)`) whenever you need an operator, a function call,
or anything the parser would otherwise read as markup. Output is HTML-escaped for you.

## `@if` / `@else if` / `@else`

`@if` takes a Motoko `Bool` expression and renders its block when the expression is true.
Chain with `@else if` and finish with `@else`.

```razor
@if count == 0 {
  <Alert type="info">Nothing here yet.</Alert>
} else if count < 5 {
  <p>Getting started — @count so far.</p>
} else {
  <p>You are on a roll: @count!</p>
}
```

Because the condition is real Motoko, you can call into your [services](services.md) or
guard on a principal:

```razor
@if Principal.isAnonymous(caller) {
  <Button kind="primary" @click="signIn">Sign in</Button>
} else {
  <Badge type="success">Signed in</Badge>
}
```

## `@for X in EXPR`

Loop over any iterable with `@for`. This is the idiomatic way to render lists — **prefer
`@for` over building HTML with `.map()`** in a code block. `@for` keeps the markup in the
template where it is readable, and it composes cleanly with components and nested control
flow.

```razor
@code {
  let products = [
    { name = "Helmet"; price = 120 },
    { name = "Gloves"; price = 45 },
  ];
}

<Grid columns="2">
  @for p in products.vals() {
    <Card title="@p.name">
      <p>$@p.price</p>
      <Button kind="primary" @click="addToCart(p.name)">Add</Button>
    </Card>
  }
</Grid>
```

Iterate an array with `.vals()`, or loop over any other Motoko iterator the same way
(for example `Iter.range(0, n)`). Handler arguments like `addToCart(p.name)` are evaluated
server-side at render time and baked into the event payload — see [Events](events.md).

Combine loops with `@if` to handle empty collections:

```razor
@if products.size() == 0 {
  <Alert type="info">No products yet.</Alert>
} else {
  @for p in products.vals() {
    <p>@p.name</p>
  }
}
```

## `@switch` over variants

When you are working with Motoko variant types, `@switch` is clearer than a chain of
`@if`s. Each `case` matches a `#Variant`, and pattern bindings are available inside the
block.

```razor
@code {
  type Status = { #Draft; #Published; #Archived : { reason : Text } };
  let status : Status = #Published;
}

@switch status {
  case #Draft {
    <Badge type="warning">Draft</Badge>
  }
  case #Published {
    <Badge type="success">Live</Badge>
  }
  case #Archived a {
    <Alert type="info">Archived: @a.reason</Alert>
  }
}
```

`@switch` is exhaustive in the same way Motoko's `switch` is — cover every variant your
type can hold so the generated Motoko compiles cleanly.

## Putting it together

Control flow nests freely. Loop over a collection, switch on each item's state, and print
values inline — all in one readable template:

```razor
@for order in orders.vals() {
  <Card title="@order.id">
    @switch order.state {
      case #Paid { <Badge type="success">Paid</Badge> }
      case #Pending { <Badge type="warning">Pending</Badge> }
    }
    <p>Total: $@(order.total)</p>
  </Card>
}
```

Build it and watch the rendered output update:

```bash
motoview dev
```

Next, learn how the buttons above turn clicks into typed Motoko calls in
[Events](events.md).
