---
title: Custom UI (A2UI)
description: Build Rich User Interfaces with AI or by Hand
sidebar:
  order: 46
---

Flow-Like supports [A2UI (Agent-to-User Interface)](https://a2ui.org/), an open protocol created by Google that enables rich, structured user interfaces to be generated dynamically.

## What is A2UI?

A2UI is a JSON-based format for describing user interfaces. It defines **pages** (full-screen layouts) and **widgets** (reusable components) that can be rendered into beautiful, interactive UIs.

The key insight: the same JSON format that AI agents produce can also be created by humans using a visual builder.

## Two Ways to Build

### 1. Visual Drag-and-Drop Builder

Flow-Like provides a visual interface builder where you can:

- **Drag and drop** components onto a canvas
- **Configure** each component's properties visually
- **Preview** your interface in real-time
- **Connect** to your Flows for dynamic data

No coding required—perfect for designers and non-technical users who want full control over their interfaces.

See **[Pages](/apps/pages/)** and **[Widgets](/apps/widgets/)** for detailed guides on using the visual builder.

### 2. AI-Generated Interfaces

Connect an AI agent to your Flow, and it can generate A2UI interfaces on-the-fly:

- **Dynamic layouts** that adapt to context
- **Personalized content** based on user data
- **Conversational UI** that evolves with the interaction

Both approaches output the same A2UI JSON format, so you can mix and match—start with a template you designed, let AI customize it, then refine it visually.

## Core Concepts

### Pages

Pages are full-screen layouts specific to your app. Each page:

- Has a unique URL path via [Routes](/apps/routes/)
- Contains components arranged in a layout
- Can bind to flow data for dynamic content
- Supports different layout types (Grid, Stack, Sidebar)

**[Learn more about Pages →](/apps/pages/)**

### Widgets

Widgets are reusable UI components that can be shared:

- Build once, use across multiple pages
- Configure with customization options per instance
- Version and share within your organization

**[Learn more about Widgets →](/apps/widgets/)**

### Routes

Routes map URL paths to pages or events:

- Define navigation structure for your app
- Support deep linking to specific content
- Can trigger events for API endpoints

**[Learn more about Routes →](/apps/routes/)**

## Supported Components

Flow-Like supports a comprehensive set of A2UI components:

| Category | Components |
|----------|------------|
| **Layout** | Row, Column, Stack, Grid, ScrollArea, AspectRatio, Overlay, Absolute |
| **Display** | Text, Image, Icon, Video, Markdown, Divider, Badge, Avatar, UserProfile, Progress, Spinner, Skeleton, Lottie, Iframe, PlotlyChart |
| **Interactive** | Button, TextField, Select, Slider, Checkbox, Switch, RadioGroup, DateTimeInput, FileInput, ImageInput, Link |
| **Container** | Card, Modal, Tabs, Accordion, Drawer, Tooltip, Popover |
| **Game/Visual** | Canvas2D, Sprite, Shape, Scene3D, Model3D, Dialogue, CharacterPortrait, ChoiceMenu, InventoryGrid, HealthBar, MiniMap |

For a complete reference, see **[A2UI Components](/reference/a2ui-components/)**.

## When to Use What

| Use Case | Recommendation |
|----------|----------------|
| Simple chat responses | Use the [Chat UI](/apps/chat-ui/) |
| Rich dashboards | Use Pages with data bindings |
| AI-generated reports | Use A2UI with AI generation |
| Interactive forms | Use Pages with form components |
| Reusable UI patterns | Create Widgets |
| Multi-page apps | Use Pages + Routes |

## Getting Started

### Create a Page

1. Open your app in Flow-Like
2. Click the **gear icon** (⚙️) → **Pages & Routes**
3. Create a new page or open the Page Builder
4. Design your interface with drag-and-drop

### Set Up Routing

1. In **Pages & Routes**, switch to the **Routes** tab
2. Click **Add Route**
3. Define the path and select your page as the target
4. Mark one route as default (home page)

### Connect to Data

1. In the Page Builder, select a component
2. Find **Data Binding** in the properties panel
3. Choose a path from your flow's data
4. The component displays live data when running

## Human + AI Collaboration

The real power of A2UI in Flow-Like is combining human creativity with AI capabilities:

1. **Design a template** using the visual builder
2. **Mark sections as dynamic** where AI can fill in content
3. **Connect to an AI Flow** that provides personalized data
4. **Users see** a polished interface with relevant content

This gives you:

- **Consistent branding** - You control the design
- **Dynamic content** - AI personalizes for each user
- **Maintainability** - Update templates, AI adapts

## AI-Powered Interface Generation

Don't want to design from scratch? Use AI to generate A2UI interfaces instantly:

### FlowPilot (Built-in)

Flow-Like includes **FlowPilot**, an integrated AI assistant that can generate A2UI interfaces directly within the app. Simply describe what you want and FlowPilot creates the components for you—no copy-pasting required.

- **Context-aware** - FlowPilot understands your app's data and existing pages
- **Interactive** - Refine and iterate on designs through conversation
- **Integrated** - Generated UI appears directly in the page builder

### ChatGPT Frontend Builder (External)

We also provide a free [**FlowLike Frontend Builder GPT**](https://chatgpt.com/g/g-6965146c7f5c81918a2501c5a860d9e3-flow-like-frontend-builder) for generating A2UI JSON outside of Flow-Like.

**Example prompts:**
- "Create a login form with email, password, and remember me checkbox"
- "Build a dashboard with 4 stat cards and a line chart"
- "Make a pricing page with 3 tier cards (Free, Pro, Enterprise)"
- "Create a user profile card with avatar, name, and edit button"

The GPT outputs JSON that you can paste directly into Flow-Like's page builder.

### Styling Rules

The generator uses **shadcn/ui theme tokens** for automatic dark/light mode support:
- **Preferred:** `bg-background`, `text-foreground`, `bg-primary`, `text-muted-foreground`
- **Also allowed:** Hardcoded colors (`bg-red-500`) when you request specific colors

:::tip[For Developers]
Building A2UI programmatically? Check the **[Developer Guide](/dev/a2ui/overview/)** for the full specification, TypeScript interfaces, and code examples.
:::
