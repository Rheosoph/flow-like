/**
 * FlowPilot A2UI Generation Prompts
 * System prompts and guidance for AI-assisted UI generation
 */

import { validateComponents } from "../../components/flowpilot/validateComponents";

export const A2UI_SYSTEM_PROMPT = `You are FlowPilot, an AI assistant specialized in generating A2UI (Adaptive Agentic UI) interfaces. A2UI is a declarative UI system that enables building dynamic, data-bound interfaces.

## Core Principles

1. **Component-Based Architecture**: Build UIs from composable components
2. **Data Binding**: Bind props to the data model with { "path": "$.field" }
3. **Style System**: Apply styles via className (Tailwind) and structured style properties
4. **Actions**: Define user interactions that trigger backend events
5. **Responsiveness**: Support multiple screen sizes with responsive overrides

## Component Format

Write every component as ONE flat object: \`id\`, \`type\`, optional \`style\`, then its props.
Props take plain JSON values:
- Text, sizes and variants: "Hello", "16px", "heading"
- Numbers and booleans: 42, true
- Lists and records: ["a", "b"], [{ "value": "a", "label": "A" }]
- Data path: { "path": "$.user.name" } (references dataModel). A bare string is always a literal.
- Children: an array of component ids, e.g. "children": ["header", "body"]

## Component Categories

### Layout Components
- **row**: Horizontal flex layout with gap, align, justify
- **column**: Vertical flex layout with gap, align, justify
- **grid**: CSS grid with columns, rows, gaps
- **stack**: Z-index stacking for overlays (MUST specify width/height or use className with sizing like "w-full h-64")
- **scrollArea**: Scrollable container
- **aspectRatio**: Maintains width/height ratio
- **absolute**: Positioned container
- **overlay**: Base component with positioned overlays

### Display Components
- **text**: Text display with variant (body | heading | label | caption | code), size, weight, color
- **image**: Image with src, alt, fit, loading
- **icon**: Lucide icon by name
- **video**: Video player
- **lottie**: Animated graphics
- **markdown**: Rendered markdown content
- **divider**: Horizontal/vertical separator
- **badge**: Status indicator
- **avatar**: User avatar
- **userProfile**: User lookup display by sub, with avatar/chip/row/detailed/card variants (the "local" sub of an unauthenticated execution renders the current user)
- **progress**: Progress bar
- **spinner**: Loading indicator
- **skeleton**: Loading placeholder

### Interactive Components
- **button**: Clickable button with variants
- **textField**: Text input with validation
- **select**: Dropdown selection
- **slider**: Range input
- **checkbox**: Boolean toggle with label
- **switch**: On/off toggle
- **radioGroup**: Single selection from options
- **dateTimeInput**: Date/time picker

### Container Components
- **card**: Content card with title, description
- **modal**: Dialog overlay
- **tabs**: Tabbed content
- **accordion**: Collapsible sections
- **drawer**: Slide-out panel
- **tooltip**: Hover information
- **popover**: Click-triggered popup

### Game/Special Components
- **canvas2d**: 2D drawing canvas
- **sprite**: Game sprite with animations
- **shape**: Vector shapes
- **scene3d**: 3D scene container
- **model3d**: 3D model display
- **dialogue**: Visual novel dialogue
- **characterPortrait**: Character image display
- **choiceMenu**: Interactive choices
- **inventoryGrid**: Item grid
- **healthBar**: HP/MP bar
- **miniMap**: Mini map display

## Style System

Use className for Tailwind classes (preferred for sizing, spacing, colors):
\`\`\`json
{
  "className": "p-4 rounded-lg bg-card border w-full max-w-md"
}
\`\`\`

For dynamic/complex styles, use structured properties:
- background: { color }, { gradient: { type, angle, direction, stops } }, { image }, or { blur }
- border: { width, style, color, radius }
- shadow: { x, y, blur, spread, color, inset }
- margin/padding: { top, right, bottom, left }
- width/height/minWidth/maxWidth/minHeight/maxHeight: CSS strings such as "100%"
- position: { type, top, right, bottom, left }
- transform: { translate, rotate, scale, skew, transformOrigin }
- overflow: "visible" | "hidden" | "scroll" | "auto"

## Response Format

Always respond with valid JSON matching the Surface structure:
\`\`\`json
{
  "rootComponentId": "root",
  "components": [
    { "id": "root", "type": "column", "gap": "16px", "style": { "className": "p-4" }, "children": ["title", "body"] },
    { "id": "title", "type": "text", "content": { "path": "$.title" }, "variant": "heading" },
    { "id": "body", "type": "text", "content": "Plain strings are literal text" }
  ],
  "dataModel": [
    { "path": "$.title", "value": "Hello World" }
  ]
}
\`\`\`

## Actions

Interactive components trigger backend events with a fixed action verb:
\`"actions": [{ "name": "workflow_event", "context": { "nodeId": "<board event node id>" } }]\`
(\`navigate_page\` takes \`context.route\`, \`external_link\` takes \`context.url\`). The name is never an
event or node name; those identifiers go in the context.
An action only INVOKES the named event — the board handler reads live input values itself
(Get Element Value / Get File Input Files on the element refs). The context carries routing ids
only; never try to forward what the user typed or selected through the action payload.

## Best Practices

1. Use semantic IDs: "header", "main-content", "submit-button"
2. Prefer Tailwind className for styling
3. Use data binding for dynamic content
4. Keep component trees shallow when possible
5. Use Row/Column for layout, Grid for complex arrangements
6. Add loading states with Skeleton or Spinner
7. Include error handling with visible feedback
8. Make interactive elements accessible`;

export const COMPONENT_SELECTION_GUIDANCE = `## Component Selection Guide

When building A2UI interfaces, choose components based on the use case:

### For Text Content
- Single line label → text with "variant": "label"
- Heading → text with "variant": "heading", "size": "xl"
- Paragraph → text with "variant": "body"
- Code snippet → text with "variant": "code"
- Rich text → markdown

### For User Input
- Single-line text → textField
- Multi-line text → textField with "multiline": true
- Number → textField with "inputType": "number"
- Password → textField with "inputType": "password"
- Email → textField with "inputType": "email"
- Single selection → select
- Several independent choices → one checkbox each
- Yes/No → switch or checkbox
- Range value → slider
- One of a few visible options → radioGroup

### For Actions
- Primary action → button with "variant": "default"
- Secondary action → button with "variant": "secondary"
- Destructive action → button with "variant": "destructive"
- Icon-only button → button with "size": "icon" and an "icon" name

### For Layout
- Horizontal items → row
- Vertical items → column
- Complex grid → grid with columns
- Overlapping content → stack (ALWAYS set "width" and "height", e.g. "width": "400px", "height": "300px", or use className="w-full h-64")
- Long content → scrollArea

### For Grouping
- Related content → card
- Switchable views → tabs
- Collapsible sections → accordion
- Modal dialog → modal
- Side content → drawer

### For Status/Feedback
- Loading → spinner or skeleton
- Progress → progress
- Status label → badge
- Help text → tooltip
- Details → popover

### For Media
- Photo → image
- Icon → icon (use Lucide names)
- Animation → lottie
- Video → video`;

export const STYLE_SUGGESTION_PROMPT = `## A2UI Styling Guidelines

### Spacing
Use Tailwind classes for spacing:
- padding: p-1 to p-16, px-*, py-*, pt-*, etc.
- margin: m-1 to m-16, mx-*, my-*, etc.
- gap: gap-1 to gap-16

### Sizing
- width: w-full, w-1/2, w-64, w-[300px]
- height: h-full, h-screen, h-64, h-[200px]
- min/max: min-w-0, max-w-md, min-h-screen

### Colors
Use semantic tokens:
- bg-background, bg-card, bg-muted
- text-foreground, text-muted-foreground
- border-border, border-input
- bg-primary, text-primary-foreground
- bg-secondary, text-secondary-foreground
- bg-destructive, text-destructive-foreground
- bg-accent, text-accent-foreground

### Typography
- text-xs, text-sm, text-base, text-lg, text-xl, text-2xl
- font-light, font-normal, font-medium, font-semibold, font-bold
- leading-tight, leading-normal, leading-relaxed
- tracking-tight, tracking-normal, tracking-wide

### Borders & Elevation
- rounded-none, rounded, rounded-md, rounded-lg, rounded-full
- border, border-2, border-t, border-b
- shadow-floating for real elevation (shadow-sm/md/lg are transparent in this theme)

### Flexbox Alignment
For row/column components, use props:
- "align": "start" (or center/end/stretch/baseline)
- "justify": "start" (or center/end/between/around/evenly)

### Responsive Design
Apply breakpoint-specific styles via responsiveOverrides:
\`\`\`json
{
  "className": "p-2",
  "responsiveOverrides": {
    "md": { "className": "p-4" },
    "lg": { "className": "p-6" }
  }
}
\`\`\``;

/** A component in the plain authoring form: flat props, plain values, child id arrays. */
interface AuthoredComponent {
	readonly id: string;
	readonly type: string;
	readonly [prop: string]: unknown;
}

interface A2UIExample {
	readonly name: string;
	readonly description: string;
	readonly input: string;
	readonly output: {
		readonly rootComponentId: string;
		readonly components: readonly AuthoredComponent[];
		readonly dataModel: readonly {
			readonly path: string;
			readonly value: unknown;
		}[];
	};
}

const toggleRow = (key: string, title: string, description: string) =>
	[
		{
			id: `${key}-toggle`,
			type: "row",
			align: "center",
			justify: "between",
			style: { className: "p-3 rounded-lg border" },
			children: [`${key}-label`, `${key}-switch`],
		},
		{
			id: `${key}-label`,
			type: "column",
			gap: "2px",
			children: [`${key}-title`, `${key}-desc`],
		},
		{
			id: `${key}-title`,
			type: "text",
			content: title,
			variant: "body",
			weight: "medium",
		},
		{
			id: `${key}-desc`,
			type: "text",
			content: description,
			variant: "caption",
			color: "text-muted-foreground",
		},
		{
			id: `${key}-switch`,
			type: "switch",
			checked: { path: `$.settings.${key}Enabled` },
		},
	] satisfies AuthoredComponent[];

export const FEW_SHOT_EXAMPLES: readonly A2UIExample[] = [
	{
		name: "Login Form",
		description: "A simple login form with email, password, and submit button",
		input: "Create a login form",
		output: {
			rootComponentId: "login-form",
			components: [
				{
					id: "login-form",
					type: "column",
					gap: "16px",
					style: {
						className: "p-6 bg-card rounded-lg border w-full max-w-sm mx-auto",
					},
					children: [
						"form-title",
						"email-field",
						"password-field",
						"submit-btn",
						"forgot-link",
					],
				},
				{
					id: "form-title",
					type: "text",
					content: "Sign In",
					variant: "heading",
					size: "xl",
					align: "center",
				},
				{
					id: "email-field",
					type: "textField",
					value: { path: "$.email" },
					placeholder: "Email address",
					label: "Email",
					inputType: "email",
				},
				{
					id: "password-field",
					type: "textField",
					value: { path: "$.password" },
					placeholder: "Password",
					label: "Password",
					inputType: "password",
				},
				{
					id: "submit-btn",
					type: "button",
					label: "Sign In",
					variant: "default",
					actions: [
						{ name: "workflow_event", context: { nodeId: "evt-sign-in" } },
					],
				},
				{
					id: "forgot-link",
					type: "button",
					label: "Forgot password?",
					variant: "link",
					actions: [
						{ name: "navigate_page", context: { route: "/forgot-password" } },
					],
				},
			],
			dataModel: [
				{ path: "$.email", value: "" },
				{ path: "$.password", value: "" },
			],
		},
	},
	{
		name: "User Card",
		description: "A card displaying user information",
		input: "Create a user profile card",
		output: {
			rootComponentId: "user-card",
			components: [
				{
					id: "user-card",
					type: "card",
					style: { className: "p-4" },
					children: ["card-header", "card-content", "card-actions"],
				},
				{
					id: "card-header",
					type: "row",
					gap: "12px",
					align: "center",
					children: ["user-avatar", "user-info"],
				},
				{
					id: "user-avatar",
					type: "avatar",
					src: { path: "$.user.avatar" },
					fallback: { path: "$.user.initials" },
					size: "lg",
				},
				{
					id: "user-info",
					type: "column",
					gap: "2px",
					children: ["user-name", "user-role"],
				},
				{
					id: "user-name",
					type: "text",
					content: { path: "$.user.name" },
					variant: "heading",
					size: "md",
					weight: "semibold",
				},
				{
					id: "user-role",
					type: "text",
					content: { path: "$.user.role" },
					variant: "caption",
					color: "text-muted-foreground",
				},
				{
					id: "card-content",
					type: "text",
					content: { path: "$.user.bio" },
					variant: "body",
					maxLines: 3,
				},
				{
					id: "card-actions",
					type: "row",
					gap: "8px",
					justify: "end",
					style: { className: "pt-2 border-t" },
					children: ["message-btn", "follow-btn"],
				},
				{
					id: "message-btn",
					type: "button",
					label: "Message",
					variant: "outline",
					size: "sm",
				},
				{
					id: "follow-btn",
					type: "button",
					label: "Follow",
					variant: "default",
					size: "sm",
				},
			],
			dataModel: [
				{ path: "$.user.name", value: "Jane Doe" },
				{ path: "$.user.role", value: "Product Designer" },
				{ path: "$.user.avatar", value: "/avatars/jane.jpg" },
				{ path: "$.user.initials", value: "JD" },
				{
					path: "$.user.bio",
					value:
						"Passionate about creating intuitive user experiences and design systems.",
				},
			],
		},
	},
	{
		name: "Data Table Row",
		description: "A table row with actions",
		input: "Create a list item for displaying an order",
		output: {
			rootComponentId: "order-row",
			components: [
				{
					id: "order-row",
					type: "row",
					align: "center",
					justify: "between",
					style: {
						className: "p-3 border-b hover:bg-muted/50 transition-colors",
					},
					children: [
						"order-info",
						"order-status",
						"order-amount",
						"order-actions",
					],
				},
				{
					id: "order-info",
					type: "column",
					gap: "2px",
					style: { className: "flex-1" },
					children: ["order-id", "order-date"],
				},
				{
					id: "order-id",
					type: "text",
					content: { path: "$.order.id" },
					variant: "body",
					weight: "medium",
				},
				{
					id: "order-date",
					type: "text",
					content: { path: "$.order.date" },
					variant: "caption",
					color: "text-muted-foreground",
				},
				{
					id: "order-status",
					type: "badge",
					content: { path: "$.order.status" },
					variant: "secondary",
				},
				{
					id: "order-amount",
					type: "text",
					content: { path: "$.order.total" },
					variant: "body",
					weight: "semibold",
				},
				{
					id: "order-actions",
					type: "row",
					gap: "4px",
					children: ["view-btn", "delete-btn"],
				},
				{
					id: "view-btn",
					type: "button",
					label: "View",
					variant: "ghost",
					size: "sm",
					icon: "eye",
				},
				{
					id: "delete-btn",
					type: "button",
					label: "",
					variant: "ghost",
					size: "icon",
					icon: "trash-2",
				},
			],
			dataModel: [
				{ path: "$.order.id", value: "#ORD-12345" },
				{ path: "$.order.date", value: "Mar 15, 2024" },
				{ path: "$.order.status", value: "Shipped" },
				{ path: "$.order.total", value: "$129.99" },
			],
		},
	},
	{
		name: "Settings Section",
		description: "A settings toggle section",
		input: "Create a notification settings panel",
		output: {
			rootComponentId: "settings-panel",
			components: [
				{
					id: "settings-panel",
					type: "column",
					gap: "16px",
					style: { className: "p-4" },
					children: ["settings-header", "settings-list"],
				},
				{
					id: "settings-header",
					type: "text",
					content: "Notification Settings",
					variant: "heading",
					size: "lg",
				},
				{
					id: "settings-list",
					type: "column",
					gap: "12px",
					children: ["email-toggle", "push-toggle", "sms-toggle"],
				},
				...toggleRow(
					"email",
					"Email Notifications",
					"Receive updates via email",
				),
				...toggleRow(
					"push",
					"Push Notifications",
					"Get notified on your device",
				),
				...toggleRow(
					"sms",
					"SMS Notifications",
					"Receive text messages for urgent updates",
				),
			],
			dataModel: [
				{ path: "$.settings.emailEnabled", value: true },
				{ path: "$.settings.pushEnabled", value: true },
				{ path: "$.settings.smsEnabled", value: false },
			],
		},
	},
];

export function buildA2UIPrompt(
	userRequest: string,
	context?: {
		existingComponents?: string[];
		dataModel?: Record<string, unknown>;
	},
): string {
	let prompt = `${A2UI_SYSTEM_PROMPT}\n\n${COMPONENT_SELECTION_GUIDANCE}\n\n${STYLE_SUGGESTION_PROMPT}\n\n`;

	prompt += "## Examples\n\n";
	for (const example of FEW_SHOT_EXAMPLES.slice(0, 2)) {
		prompt += `### ${example.name}\nUser: ${example.input}\nAssistant:\n\`\`\`json\n${JSON.stringify(example.output, null, 2)}\n\`\`\`\n\n`;
	}

	if (context?.existingComponents?.length) {
		prompt += `## Existing Components in Surface\nThe following component IDs already exist: ${context.existingComponents.join(", ")}\nAvoid ID collisions.\n\n`;
	}

	if (context?.dataModel) {
		prompt += `## Available Data Model\n\`\`\`json\n${JSON.stringify(context.dataModel, null, 2)}\n\`\`\`\nYou can bind to these paths.\n\n`;
	}

	prompt += `## User Request\n${userRequest}\n\nGenerate the A2UI JSON for this request. Respond ONLY with valid JSON matching the Surface format.`;

	return prompt;
}

export function parseA2UIResponse(response: string): {
	rootComponentId: string;
	components: unknown[];
	dataModel: unknown[];
} | null {
	try {
		const jsonMatch = response.match(/```json([\s\S]*?)```/);
		const jsonStr = jsonMatch ? jsonMatch[1] : response;

		const parsed = JSON.parse(jsonStr.trim());

		if (!parsed.rootComponentId || !Array.isArray(parsed.components)) {
			console.error("Invalid A2UI response structure");
			return null;
		}

		// The same validation used for AI-emitted surfaces; it lowers the plain authoring form
		// (flat components, plain values, child id arrays) to canonical components first.
		const normalizedComponents = validateComponents(
			parsed.components,
		).components;

		return {
			rootComponentId: parsed.rootComponentId,
			components: normalizedComponents,
			dataModel: parsed.dataModel || [],
		};
	} catch (error) {
		console.error("Failed to parse A2UI response:", error);
		return null;
	}
}

export const A2UI_EDIT_PROMPT = `You are editing an existing A2UI surface. Apply the user's requested changes while preserving the existing structure.

Rules:
1. Keep existing component IDs unless explicitly asked to change them
2. Preserve data bindings that are still relevant
3. Only modify the specific components mentioned
4. Return the COMPLETE updated components array (not just changes)
5. If adding new components, generate appropriate IDs
6. Write new or changed components in the flat format with plain values`;
