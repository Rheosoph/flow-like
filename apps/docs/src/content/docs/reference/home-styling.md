---
title: Home widget styling
description: Style Home widgets with Tailwind classes, scoped CSS, and theme variables.
---

Use this reference after [creating a Home layout](/start/home/). These controls
style Profile Home widgets. A2UI Widgets and native system widgets have their
own authoring models; see [widget types](/apps/overview/#four-meanings-of-widget).

## Style a widget with Tailwind classes

To go beyond the built-in surfaces and accents, select a widget and enter
Tailwind CSS utility classes in **Tailwind classes**, below **Accent** in its
settings. Separate classes with spaces. In **Customize → Layout options → Edit
JSON**, the same value is the widget's `appearance.className`.

The classes apply to the widget's surface after its surface and accent style,
so your background, border, radius, shadow, and text color replace the
built-in ones. Home compiles the classes at runtime with Tailwind CSS v4, so
arbitrary values such as `p-[18px]` work, and scopes them to that widget. The
value can be up to 1024 bytes.

The layout controls the surface's position, grid span, height, and alignment,
so `absolute`, `relative`, `sticky`, `z-*`, `col-span-*`, `row-start-*`, and
`self-*` have no effect on the surface itself. They do work behind variants
that target other elements, such as `before:absolute` or `[&_img]:relative`.
Leave out `fixed` everywhere, because it escapes the widget. The toolbar and
resize handle you see while editing sit outside the surface, so your classes
never reach them.

For example:

- Highlighted KPI card: `rounded-3xl border-primary/40 bg-primary/10`
- Gradient that follows the accent:
  `border-(--home-accent)/40 bg-linear-to-br from-(--home-accent)/15 to-transparent`
- Raised card: `rounded-2xl shadow-[0_12px_32px_-16px_rgb(0_0_0/0.35)]`
- Brand card on a dark fill:
  `bg-[#0b1f3a] text-white [--muted-foreground:rgb(255_255_255/0.7)]`
- Accent stripe along the top:
  `before:absolute before:inset-x-0 before:top-0 before:h-1 before:bg-(--home-accent)`
- Larger title on a **Borderless** surface: `[&_h2]:text-2xl`

Some tips:

- Style elements inside the widget with arbitrary variants, such as
  `[&_h2]:text-lg`, and add decoration with `before:` and `after:`. See
  [Target parts of a widget](#target-parts-of-a-widget) for reliable selectors.
- Use Tailwind CSS v4 class names, for example `bg-linear-to-br` rather than
  `bg-gradient-to-br`.
- The theme's shadow scale, `shadow-2xs` to `shadow-2xl`, is transparent. For
  elevation, use an arbitrary shadow that includes its color, like the raised
  card above.
- A text color on the surface only reaches text that inherits it.
  Descriptions, dates, and other details use `text-muted-foreground`. On a dark
  or saturated fill, set `--muted-foreground` as well, as in the brand card
  above, so that text stays readable.
- Animation classes such as `animate-in`, `fade-in`, and `slide-in-from-*` are
  not compiled at runtime. Write `@keyframes` in **Custom CSS** instead.
- Unless you want a branded look, prefer theme tokens such as `bg-card`,
  `text-primary`, and `border-primary/30`, and the
  [accent variables](#use-theme-and-accent-colors), so the widget still looks
  right in light and dark mode and follows its **Accent** setting. Keep one
  style across widgets rather than decorating each card differently.

## Style a widget with custom CSS

When utility classes are not enough, for example for an animation, a
pseudo-element, or several rules for elements inside the widget, write a
stylesheet in **Custom CSS**, below **Tailwind classes**. In **Edit JSON**, the
same value is the widget's `appearance.css`. It can be up to 8 KiB.

This is plain CSS, not Tailwind. `@apply`, `@tailwind`, `@theme`, `@variant`,
`@utility`, `theme()`, and `--alpha()` do nothing here; put utilities in
**Tailwind classes** instead.

The stylesheet only reaches its own widget:

- `:root` is the widget's surface. For dark mode, write `:root:is(.dark *)`.
- Every other selector, such as `h2` or `.my-class`, matches elements inside
  the widget.
- Rules here override the widget's Tailwind classes.
- `@keyframes` names stay local to the widget, so they cannot replace another
  widget's or the app's animations. Wrap animations in
  `@media (prefers-reduced-motion: no-preference)` so they stay still for
  people who reduce motion.

```css
@keyframes glow {
  to { box-shadow: 0 0 24px color-mix(in oklab, var(--home-accent) 40%, transparent); }
}

:root {
  background: linear-gradient(135deg, var(--card), color-mix(in oklab, var(--primary) 12%, var(--card)));
  box-shadow: 0 12px 32px -16px rgb(0 0 0 / 0.35);
}

@media (prefers-reduced-motion: no-preference) {
  :root { animation: glow 3s ease-in-out infinite alternate; }
}

h2::after { content: " ✦"; color: var(--primary); }
```

The surface is positioned and clips its overflow, so `:root::before` and
`:root::after` can use `content`, `position: absolute`, `inset`, `z-index`,
`width`, and `height` for decoration. To place a layer behind the content with
`z-index: -1`, set `isolation: isolate` on `:root`:

```css
:root { isolation: isolate; }

:root::before {
  content: "";
  position: absolute;
  inset: auto -15% -45% auto;
  width: 60%;
  aspect-ratio: 1;
  z-index: -1;
  border-radius: 50%;
  background: radial-gradient(color-mix(in oklab, var(--home-accent) 30%, transparent), transparent 70%);
}
```

Avoid these, because they reach outside the widget or conflict with the
layout. FlowPilot warns about them when it validates a layout.

- `+` or `~` after `:root`. They style the widgets next to this one.
- `@property`, `@font-face`, `@counter-style`, and `@page`. They apply to the
  whole page. `@import` is removed.
- `position`, `inset`, `z-index`, grid placement, `order`, `width`, or `height`
  on `:root` itself. The layout controls where the widget sits and how large it
  is. `:root::before` and `:root::after` may use them.
- `position: fixed` anywhere. It escapes the widget.

## Use theme and accent colors

Theme colors are CSS variables that hold complete colors: `--background`,
`--foreground`, `--card`, `--card-foreground`, `--primary`,
`--primary-foreground`, `--secondary`, `--muted`, `--muted-foreground`,
`--accent`, `--border`, and `--chart-1` to `--chart-5`. Use them as they are,
such as `var(--primary)`, or blend them with
`color-mix(in oklab, var(--primary) 30%, transparent)`. Don't wrap them in
`hsl()`, `rgb()`, or `oklch()`; `hsl(var(--primary))` is not a valid color.

Each widget also sets accent variables from its **Surface** and **Accent**
settings. Use them so your styling follows the accent when someone changes it:

| Variable | Holds |
| --- | --- |
| `--home-accent` | The accent color |
| `--home-accent-foreground` | Text color on an accent fill |
| `--home-surface-background` | The surface fill |
| `--home-surface-foreground` | Text on the surface |
| `--home-surface-muted` | Secondary text on the surface |
| `--home-surface-accent` | Accent color that stays readable on the surface |
| `--home-surface-border` | Borders on the surface |
| `--home-surface-item` | Rows and tiles inside the widget |
| `--home-surface-item-hover` | Rows and tiles on hover |

In classes, write `text-(--home-accent)`, `border-(--home-accent)/40`, or
`from-(--home-accent)/15`. In CSS, write
`color-mix(in oklab, var(--home-accent) 20%, transparent)`.

To retint a widget's own content, redefine a variable on its surface:
`[--home-accent:#0ea5e9]` in **Tailwind classes**, or
`:root { --home-accent: #0ea5e9; }` in **Custom CSS**. On a dark or saturated
fill, `:root { --muted-foreground: color-mix(in oklab, var(--primary-foreground) 75%, transparent); }`
keeps descriptions and dates readable.

## Target parts of a widget

In **Custom CSS**, use these selectors as written. In **Tailwind classes**, wrap
them in an arbitrary variant and write `_` for spaces, such as
`[&>div>header>h2]:text-xl` or `[&_[data-home-greeting]_h1]:text-4xl`.

| Part | Selector |
| --- | --- |
| Widget title, including its icon | `:root > div > header > h2` |
| Title icon | `:root > div > header > h2 > svg` |
| Widget description | `:root > div > header > p` |
| Greeting headline and line below it | `[data-home-greeting] h1`, `[data-home-greeting] p` |
| Section heading | `[data-home-section-heading] h2` |
| Spotlights, rankings, and featured collections | `[data-home-discovery] h2` |
| Package cards | `[data-package-card]` |
| Workspace pulse | `[data-workspace-pulse]` |
| Data widget chart, table, or value | `[data-home-data-presentation]` |
| Data source note | `details > summary` |

The widget title and description appear only when you set them, and not on
widgets that draw their own heading: greetings, section headings, FlowPilot,
quick actions, embedded Apps, spotlights, rankings, featured collections, and
workspace pulse. Prefer these selectors over a bare `header` or `h2`, which can
also match headings inside the widget's content.

## Ask FlowPilot to restyle your Home

Ask FlowPilot to restyle your Home, for example "turn my stats widgets into
navy cards with a soft shadow and our brand orange #ff5a1f". It writes the
Tailwind classes and custom CSS for you and uses any colors, classes, or CSS you
give it as written. It validates the CSS and warns about rules that reach
outside a widget.
