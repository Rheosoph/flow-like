# A2UI Styling Guide

This guide covers Tailwind CSS usage, shadcn/ui theming, and responsive design patterns for A2UI components.

Contents: [theme variables](#theme-variables-required), [component styles](#component-style-property), [responsive design](#responsive-design), [mobile checklist](#mobile-ready-ui-checklist), [layout utilities](#flexbox-utilities), and [custom CSS](#custom-css-injection).

---

## Theme Variables (Required)

A2UI uses shadcn/ui theme variables for consistent light/dark theming. Use semantic theme tokens instead of fixed palette colors.

### Background Colors

| Variable | Usage |
|----------|-------|
| `bg-background` | Main page background |
| `bg-foreground` | Inverted background (rare) |
| `bg-card` | Card backgrounds |
| `bg-muted` | Subtle backgrounds, disabled states |
| `bg-primary` | Primary action backgrounds |
| `bg-secondary` | Secondary action backgrounds |
| `bg-accent` | Accent/highlight backgrounds |
| `bg-destructive` | Error/danger backgrounds |

### Text Colors

| Variable | Usage |
|----------|-------|
| `text-foreground` | Primary text |
| `text-muted-foreground` | Secondary/helper text |
| `text-primary-foreground` | Text on primary background |
| `text-secondary-foreground` | Text on secondary background |
| `text-accent-foreground` | Text on accent background |
| `text-destructive-foreground` | Text on destructive background |
| `text-destructive` | Error text on light background |

### Border Colors

| Variable | Usage |
|----------|-------|
| `border-border` | Default borders |
| `border-input` | Input field borders |
| `ring-ring` | Focus rings |

### Avoid Fixed Palette Colors

These break light/dark theme adaptation:

```
bg-white, bg-black, bg-gray-*, bg-slate-*, bg-red-*, bg-blue-*, etc.
text-white, text-black, text-gray-*, text-slate-*, text-red-*, etc.
border-gray-*, border-slate-*, etc.
```

Express intent with semantic tokens: use `bg-destructive` for a destructive red action, `bg-primary` for a primary action, and `text-destructive` for error text.

---

## Component Style Property

Every component has a `style` property with `className`:

```jsonc
{
  "id": "my-component",
  "style": {
    "className": "p-4 rounded-lg bg-card border border-border"
  },
  "component": { ... }
}
```

Prefer `className` for ordinary layout and appearance. The raw `style` object also supports structured values when Tailwind is not appropriate:

```json
{
  "style": {
    "background": {
      "gradient": {
        "type": "linear",
        "angle": 135,
        "stops": [
          { "color": "hsl(var(--primary))", "position": 0 },
          { "color": "hsl(var(--accent))", "position": 100 }
        ]
      }
    },
    "border": {
      "width": "1px",
      "style": "solid",
      "color": "hsl(var(--border))",
      "radius": "0.75rem"
    },
    "shadow": {
      "x": "0",
      "y": "8px",
      "blur": "24px",
      "spread": "0",
      "color": "hsl(var(--foreground) / 0.08)",
      "inset": false
    },
    "padding": {
      "top": "1rem",
      "right": "1rem",
      "bottom": "1rem",
      "left": "1rem"
    },
    "width": "100%",
    "position": {
      "type": "relative"
    },
    "overflow": "hidden"
  }
}
```

Other supported raw style fields include `margin`, `gap`, min/max sizing, flex/grid item placement, typography, visibility, opacity, cursor, stacking, transition, animation, display, outline, filters, `aspectRatio`, and transforms (`translate`, numeric `rotate`, `scale`, `skew`, `transformOrigin`).

For structured breakpoint overrides, use `responsiveOverrides`:

```json
{
  "style": {
    "className": "p-3",
    "responsiveOverrides": {
      "md": { "className": "p-5", "gridCols": 2 },
      "lg": { "className": "p-6", "gridCols": 3 }
    }
  }
}
```

Supported keys are `sm`, `md`, `lg`, `xl`, and `xxl`. Tailwind classes remain the preferred responsive mechanism.

---

## Spacing

### Padding
| Class | Value |
|-------|-------|
| `p-0` | 0 |
| `p-1` | 0.25rem |
| `p-2` | 0.5rem |
| `p-3` | 0.75rem |
| `p-4` | 1rem |
| `p-5` | 1.25rem |
| `p-6` | 1.5rem |
| `p-8` | 2rem |
| `p-10` | 2.5rem |
| `p-12` | 3rem |

Directional: `px-*`, `py-*`, `pt-*`, `pr-*`, `pb-*`, `pl-*`

### Margin
Same scale: `m-*`, `mx-*`, `my-*`, `mt-*`, `mr-*`, `mb-*`, `ml-*`

### Gap (for flex/grid)
Use the component's `gap` prop, or `gap-*` in className for custom layouts.

---

## Sizing

### Width
| Class | Value |
|-------|-------|
| `w-full` | 100% |
| `w-auto` | auto |
| `w-screen` | 100vw |
| `w-1/2` | 50% |
| `w-1/3` | 33.333% |
| `w-2/3` | 66.666% |
| `w-1/4` | 25% |
| `w-3/4` | 75% |
| `w-fit` | fit-content |
| `w-max` | max-content |
| `w-min` | min-content |

Fixed: `w-64` (16rem), `w-96` (24rem), etc.

### Height
Same patterns: `h-full`, `h-screen`, `h-auto`, `h-fit`, `h-64`, etc.

### Max/Min
`max-w-sm`, `max-w-md`, `max-w-lg`, `max-w-xl`, `max-w-2xl`, `max-w-full`
`min-h-screen`, `min-h-full`, `min-w-0`

---

## Borders & Rounded Corners

### Border Width
| Class | Value |
|-------|-------|
| `border` | 1px |
| `border-0` | 0 |
| `border-2` | 2px |
| `border-4` | 4px |

Directional: `border-t`, `border-r`, `border-b`, `border-l`

### Border Radius
| Class | Result |
|-------|--------|
| `rounded-none` | 0 |
| `rounded-sm` | 0.125rem |
| `rounded` | 0.25rem |
| `rounded-md` | 0.375rem |
| `rounded-lg` | 0.5rem |
| `rounded-xl` | 0.75rem |
| `rounded-2xl` | 1rem |
| `rounded-full` | 9999px |

---

## Shadows

| Class | Usage |
|-------|-------|
| `shadow-sm` | Subtle shadow |
| `shadow` | Default shadow |
| `shadow-md` | Medium shadow |
| `shadow-lg` | Large shadow |
| `shadow-xl` | Extra large shadow |
| `shadow-none` | Remove shadow |

---

## Responsive Design

A2UI follows mobile-first design. Base styles apply to mobile, then override for larger screens.

### Breakpoints

| Prefix | Min Width | Target |
|--------|-----------|--------|
| (none) | 0 | Mobile |
| `sm:` | 640px | Large phones |
| `md:` | 768px | Tablets |
| `lg:` | 1024px | Laptops |
| `xl:` | 1280px | Desktops |
| `2xl:` | 1536px | Large desktops |

### Common Patterns

#### Responsive Grid
```json
{
  "style": { "className": "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4" }
}
```

#### Responsive Flex Direction
```json
{
  "style": { "className": "flex flex-col md:flex-row gap-4" }
}
```

#### Hide/Show on Breakpoints
```jsonc
// Hide on mobile, show on md+
{ "className": "hidden md:block" }

// Show on mobile, hide on md+
{ "className": "block md:hidden" }
```

#### Responsive Text
```json
{ "className": "text-sm md:text-base lg:text-lg" }
```

#### Responsive Padding
```json
{ "className": "p-4 md:p-6 lg:p-8" }
```

#### Responsive Width
```json
{ "className": "w-full md:w-1/2 lg:w-1/3" }
```

---

## Mobile-Ready UI Checklist

Use these rules for every generated interface:

1. **Start with the phone layout** - Base classes should produce a complete mobile UI. Add `sm:`, `md:`, `lg:`, `xl:`, and `2xl:` only to enhance larger screens.
2. **Always include a root wrapper** - The top-level component must be `id: "root"` and usually needs `min-h-screen w-full bg-background text-foreground`.
3. **Prevent horizontal overflow** - Use `w-full`, `max-w-full`, `min-w-0`, `overflow-hidden`, `break-words`, and responsive grid columns.
4. **Avoid fixed desktop widths** - Prefer `max-w-* mx-auto` for centered content and `w-full` for forms, cards, charts, maps, images, and tables.
5. **Use responsive grids** - Good defaults are `grid-cols-1 sm:grid-cols-2 lg:grid-cols-3` or `repeat(auto-fit, minmax(220px, 1fr))`.
6. **Give complex content stable dimensions** - Charts, maps, iframes, videos, and 3D scenes should have `height`, `min-h-*`, or an aspect ratio.
7. **Make controls touch-friendly** - Buttons, links, checkboxes, inputs, and select triggers should generally use `h-10`, `px-3`, or generous padding.
8. **Keep text resilient** - Use `leading-snug`, `break-words`, `truncate`, `maxLines`, or `text-sm md:text-base` where labels or values can be long.
9. **Design for scanning** - Put primary actions near the relevant content, use muted helper text, keep labels visible, and group repeated items consistently.
10. **Respect theme tokens** - Use `bg-background`, `bg-card`, `text-foreground`, `text-muted-foreground`, and `border-border` so light and dark mode both work.

---

## Typography

### Font Size
| Class | Size |
|-------|------|
| `text-xs` | 0.75rem |
| `text-sm` | 0.875rem |
| `text-base` | 1rem |
| `text-lg` | 1.125rem |
| `text-xl` | 1.25rem |
| `text-2xl` | 1.5rem |
| `text-3xl` | 1.875rem |
| `text-4xl` | 2.25rem |

### Font Weight
`font-light`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`

### Line Height
`leading-none`, `leading-tight`, `leading-snug`, `leading-normal`, `leading-relaxed`, `leading-loose`

### Text Alignment
`text-left`, `text-center`, `text-right`, `text-justify`

---

## Flexbox Utilities

| Class | Effect |
|-------|--------|
| `flex` | Enable flex |
| `flex-row` | Horizontal (default) |
| `flex-col` | Vertical |
| `flex-wrap` | Allow wrapping |
| `flex-nowrap` | Prevent wrapping |
| `flex-1` | Grow and shrink |
| `flex-none` | Don't grow or shrink |
| `flex-grow` | Grow to fill |
| `flex-shrink-0` | Don't shrink |

### Alignment
| Class | Effect |
|-------|--------|
| `items-start` | Align to start |
| `items-center` | Center align |
| `items-end` | Align to end |
| `items-stretch` | Stretch to fill |
| `justify-start` | Pack to start |
| `justify-center` | Center pack |
| `justify-end` | Pack to end |
| `justify-between` | Space between |
| `justify-around` | Space around |
| `justify-evenly` | Space evenly |

---

## Common Component Styling Patterns

### Card with Shadow
```json
{
  "style": { "className": "bg-card rounded-lg border border-border shadow-sm p-6" }
}
```

### Input Container
```json
{
  "style": { "className": "space-y-2" }
}
```

### Full-width Button
```json
{
  "style": { "className": "w-full" }
}
```

### Centered Container
```json
{
  "style": { "className": "max-w-md mx-auto" }
}
```

### Header Bar
```json
{
  "style": { "className": "border-b border-border px-4 py-3" }
}
```

### Sticky Header
```json
{
  "style": { "className": "sticky top-0 z-10 bg-background/95 backdrop-blur" }
}
```

### Grid of Cards
```json
{
  "style": { "className": "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6" }
}
```

### Sidebar Layout
```jsonc
// Container
{ "className": "flex flex-col md:flex-row min-h-screen" }

// Sidebar
{ "className": "w-full md:w-64 bg-muted border-r border-border" }

// Main content
{ "className": "flex-1 p-6" }
```

---

## Animation Classes

| Class | Effect |
|-------|--------|
| `animate-spin` | Continuous rotation |
| `animate-ping` | Ping effect |
| `animate-pulse` | Fade in/out |
| `animate-bounce` | Bouncing |
| `transition` | Enable transitions |
| `duration-150` | 150ms duration |
| `duration-300` | 300ms duration |
| `ease-in-out` | Smooth easing |

### Hover Effects
```jsonc
{ "className": "hover:bg-muted transition-colors" }
{ "className": "hover:shadow-md transition-shadow" }
{ "className": "hover:scale-105 transition-transform" }
```

---

## Z-Index

| Class | Value |
|-------|-------|
| `z-0` | 0 |
| `z-10` | 10 |
| `z-20` | 20 |
| `z-30` | 30 |
| `z-40` | 40 |
| `z-50` | 50 |
| `z-auto` | auto |

---

## Overflow

| Class | Effect |
|-------|--------|
| `overflow-auto` | Scroll when needed |
| `overflow-hidden` | Clip overflow |
| `overflow-scroll` | Always show scrollbar |
| `overflow-visible` | Show overflow |
| `overflow-x-auto` | Horizontal scroll |
| `overflow-y-auto` | Vertical scroll |

---

## Position

| Class | Effect |
|-------|--------|
| `relative` | Relative positioning |
| `absolute` | Absolute positioning |
| `fixed` | Fixed positioning |
| `sticky` | Sticky positioning |
| `top-0`, `right-0`, `bottom-0`, `left-0` | Position offsets |
| `inset-0` | All sides 0 |

---

## Custom CSS Injection

For advanced styling not achievable with Tailwind, use `canvasSettings.customCss`. The CSS is automatically scoped to the page container.

### Format

```json
{
  "canvasSettings": {
    "backgroundColor": "bg-background",
    "padding": "1rem",
    "customCss": ".my-element { animation: pulse 2s infinite; } @keyframes pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.5; } }"
  }
}
```

### Good Use Cases

| Use Case | Example |
|----------|--------|
| Custom animations | `@keyframes`, `animation` properties |
| Complex gradients | `background: linear-gradient(...)` with multiple stops |
| Pseudo-elements | `::before`, `::after` for decorative effects |
| Advanced hover states | Complex `:hover` transitions |
| CSS variables | `--custom-color: #xxx` for theming |
| Filters & effects | `backdrop-filter`, `filter`, `mix-blend-mode` |

### Examples

#### Pulsing Glow Effect
```css
.glow-card {
  animation: glow 2s ease-in-out infinite;
}
@keyframes glow {
  0%, 100% { box-shadow: 0 0 5px hsl(var(--primary)); }
  50% { box-shadow: 0 0 20px hsl(var(--primary)); }
}
```

#### Gradient Text
```css
.gradient-text {
  background: linear-gradient(90deg, hsl(var(--primary)), hsl(var(--accent)));
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}
```

#### Glass Morphism
```css
.glass {
  background: hsl(var(--background) / 0.7);
  backdrop-filter: blur(10px);
  border: 1px solid hsl(var(--border) / 0.3);
}
```

#### Animated Gradient Background
```css
.animated-bg {
  background: linear-gradient(-45deg, hsl(var(--primary)), hsl(var(--accent)), hsl(var(--secondary)), hsl(var(--muted)));
  background-size: 400% 400%;
  animation: gradient-shift 15s ease infinite;
}
@keyframes gradient-shift {
  0% { background-position: 0% 50%; }
  50% { background-position: 100% 50%; }
  100% { background-position: 0% 50%; }
}
```

### Best Practices

1. **Prefer Tailwind first** - Only use customCss when Tailwind can't achieve the effect
2. **Use theme variables** - Reference `hsl(var(--primary))` instead of hardcoded colors
3. **Keep it minimal** - Don't duplicate what Tailwind can do
4. **Use descriptive class names** - `.hero-glow`, `.card-shimmer`, not `.c1`, `.s2`
5. **Test responsiveness** - Custom CSS should work at all breakpoints
