/**
 * FlowPilot A2UI Generation Prompts
 * System prompts and guidance for AI-assisted UI generation
 */

import { validateComponents } from "../../components/flowpilot/validateComponents";

export const A2UI_SYSTEM_PROMPT = `You are FlowPilot, an AI assistant specialized in generating A2UI (Adaptive Agentic UI) interfaces. A2UI is a declarative UI system that enables building dynamic, data-bound interfaces.

## Core Principles

1. **Component-Based Architecture**: Build UIs from composable components
2. **Data Binding**: Use BoundValue to connect UI to dynamic data
3. **Style System**: Apply styles via className (Tailwind) and structured style properties
4. **Actions**: Define user interactions that trigger backend events
5. **Responsiveness**: Support multiple screen sizes with responsive overrides

## BoundValue Format

Every component prop except \`type\` and structural containers such as
\`children.explicitList\` must use BoundValue, including static variants, sizes, booleans, and
numbers. Data can be bound using either literal values or data paths:
- Literal string: { "literalString": "Hello" }
- Literal number: { "literalNumber": 42 }
- Literal boolean: { "literalBool": true }
- Data path: { "path": "$.user.name" } (references dataModel)

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
- **text**: Text display with variant, size, weight, color
- **image**: Image with src, alt, fit, loading
- **icon**: Lucide icon by name
- **video**: Video player
- **lottie**: Animated graphics
- **markdown**: Rendered markdown content
- **divider**: Horizontal/vertical separator
- **badge**: Status indicator
- **avatar**: User avatar
- **userProfile**: User lookup display by sub, with avatar/chip/row/detailed/card variants
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
- **canvas2D**: 2D drawing canvas
- **sprite**: Game sprite with animations
- **shape**: Vector shapes
- **scene3D**: 3D scene container
- **model3D**: 3D model display
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
  "className": "p-4 rounded-lg bg-card shadow-md w-full max-w-md"
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
  "rootComponentId": "root-id",
  "components": [
    {
      "id": "root-id",
      "style": { "className": "..." },
      "component": {
        "type": "column",
        "gap": { "literalString": "16px" },
        "children": { "explicitList": ["child-1", "child-2"] }
      }
    }
  ],
  "dataModel": [
    { "path": "$.title", "value": "Hello World" }
  ]
}
\`\`\`

## Actions

Interactive components trigger backend events via \`actions: [{ "name": "eventName", "context": {...} }]\`.
An action only INVOKES the named event — the board handler reads live input values itself
(Get Element Value / Get File Input Files on the element refs). Use \`context\` for static
identity data only (which form, which row id); never try to forward what the user typed or
selected through the action payload.

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
- Single line label → text with variant={"literalString":"label"}
- Heading → text with variant={"literalString":"heading"}, size={"literalString":"xl"}
- Paragraph → text with variant={"literalString":"body"}
- Code snippet → text with variant={"literalString":"code"}
- Rich text → markdown

### For User Input
- Single-line text → textField
- Multi-line text → textField with multiline={"literalBool":true}
- Number → textField with inputType={"literalString":"number"}
- Password → textField with inputType={"literalString":"password"}
- Email → textField with inputType={"literalString":"email"}
- Single selection → select
- Multiple selection → select with multiple={"literalBool":true}
- Yes/No → switch or checkbox
- Range value → slider
- Multiple options → radioGroup

### For Actions
- Primary action → button with variant={"literalString":"default"}
- Secondary action → button with variant={"literalString":"secondary"}
- Destructive action → button with variant={"literalString":"destructive"}
- Icon-only button → button with size={"literalString":"icon"}

### For Layout
- Horizontal items → row
- Vertical items → column
- Complex grid → grid with columns
- Overlapping content → stack (ALWAYS set width and height as BoundValues, e.g., width={"literalString":"400px"} height={"literalString":"300px"}, or use className="w-full h-64")
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

### Borders & Shadows
- rounded-none, rounded, rounded-md, rounded-lg, rounded-full
- border, border-2, border-t, border-b
- shadow-sm, shadow, shadow-md, shadow-lg

### Flexbox Alignment
For row/column components, use props:
- align: {"literalString":"start"} (or center/end/stretch)
- justify: {"literalString":"start"} (or center/end/between/around)

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

export const FEW_SHOT_EXAMPLES = [
	{
		name: "Login Form",
		description: "A simple login form with email, password, and submit button",
		input: "Create a login form",
		output: {
			rootComponentId: "login-form",
			components: [
				{
					id: "login-form",
					style: {
						className:
							"flex flex-col gap-4 p-6 bg-card rounded-lg shadow-md w-full max-w-sm mx-auto",
					},
					component: {
						type: "column" as const,
						gap: "16px",
						children: {
							explicitList: [
								"form-title",
								"email-field",
								"password-field",
								"submit-btn",
								"forgot-link",
							],
						},
					},
				},
				{
					id: "form-title",
					component: {
						type: "text" as const,
						content: { literalString: "Sign In" },
						variant: "heading" as const,
						size: "xl" as const,
						align: "center" as const,
					},
				},
				{
					id: "email-field",
					component: {
						type: "textField" as const,
						value: { path: "$.email" },
						placeholder: { literalString: "Email address" },
						label: "Email",
						inputType: "email" as const,
					},
				},
				{
					id: "password-field",
					component: {
						type: "textField" as const,
						value: { path: "$.password" },
						placeholder: { literalString: "Password" },
						label: "Password",
						inputType: "password" as const,
					},
				},
				{
					id: "submit-btn",
					component: {
						type: "button" as const,
						label: { literalString: "Sign In" },
						variant: "default" as const,
						actions: [{ name: "submit", context: { form: "login" } }],
					},
				},
				{
					id: "forgot-link",
					component: {
						type: "button" as const,
						label: { literalString: "Forgot password?" },
						variant: "link" as const,
						actions: [
							{ name: "navigate", context: { to: "/forgot-password" } },
						],
					},
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
					style: {
						className:
							"flex flex-col gap-3 p-4 bg-card rounded-lg shadow border",
					},
					component: {
						type: "card" as const,
						children: {
							explicitList: ["card-header", "card-content", "card-actions"],
						},
					},
				},
				{
					id: "card-header",
					style: { className: "flex items-center gap-3" },
					component: {
						type: "row" as const,
						gap: "12px",
						align: "center" as const,
						children: { explicitList: ["user-avatar", "user-info"] },
					},
				},
				{
					id: "user-avatar",
					component: {
						type: "avatar" as const,
						src: { path: "$.user.avatar" },
						fallback: { path: "$.user.initials" },
						size: "lg" as const,
					},
				},
				{
					id: "user-info",
					component: {
						type: "column" as const,
						gap: "2px",
						children: { explicitList: ["user-name", "user-role"] },
					},
				},
				{
					id: "user-name",
					component: {
						type: "text" as const,
						content: { path: "$.user.name" },
						variant: "heading" as const,
						size: "md" as const,
						weight: "semibold" as const,
					},
				},
				{
					id: "user-role",
					component: {
						type: "text" as const,
						content: { path: "$.user.role" },
						variant: "caption" as const,
						color: "text-muted-foreground",
					},
				},
				{
					id: "card-content",
					component: {
						type: "text" as const,
						content: { path: "$.user.bio" },
						variant: "body" as const,
						maxLines: 3,
					},
				},
				{
					id: "card-actions",
					style: { className: "flex gap-2 pt-2 border-t" },
					component: {
						type: "row" as const,
						gap: "8px",
						justify: "end" as const,
						children: { explicitList: ["message-btn", "follow-btn"] },
					},
				},
				{
					id: "message-btn",
					component: {
						type: "button" as const,
						label: { literalString: "Message" },
						variant: "outline" as const,
						size: "sm" as const,
					},
				},
				{
					id: "follow-btn",
					component: {
						type: "button" as const,
						label: { literalString: "Follow" },
						variant: "default" as const,
						size: "sm" as const,
					},
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
					style: {
						className:
							"flex items-center justify-between p-3 border-b hover:bg-muted/50 transition-colors",
					},
					component: {
						type: "row" as const,
						align: "center" as const,
						justify: "between" as const,
						children: {
							explicitList: [
								"order-info",
								"order-status",
								"order-amount",
								"order-actions",
							],
						},
					},
				},
				{
					id: "order-info",
					style: { className: "flex-1" },
					component: {
						type: "column" as const,
						gap: "2px",
						children: { explicitList: ["order-id", "order-date"] },
					},
				},
				{
					id: "order-id",
					component: {
						type: "text" as const,
						content: { path: "$.order.id" },
						variant: "body" as const,
						weight: "medium" as const,
					},
				},
				{
					id: "order-date",
					component: {
						type: "text" as const,
						content: { path: "$.order.date" },
						variant: "caption" as const,
						color: "text-muted-foreground",
					},
				},
				{
					id: "order-status",
					component: {
						type: "badge" as const,
						content: { path: "$.order.status" },
						variant: "secondary" as const,
					},
				},
				{
					id: "order-amount",
					component: {
						type: "text" as const,
						content: { path: "$.order.total" },
						variant: "body" as const,
						weight: "semibold" as const,
					},
				},
				{
					id: "order-actions",
					component: {
						type: "row" as const,
						gap: "4px",
						children: { explicitList: ["view-btn", "delete-btn"] },
					},
				},
				{
					id: "view-btn",
					component: {
						type: "button" as const,
						label: { literalString: "View" },
						variant: "ghost" as const,
						size: "sm" as const,
						icon: "eye",
					},
				},
				{
					id: "delete-btn",
					component: {
						type: "button" as const,
						label: { literalString: "" },
						variant: "ghost" as const,
						size: "icon" as const,
						icon: "trash-2",
					},
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
					style: { className: "flex flex-col gap-4 p-4" },
					component: {
						type: "column" as const,
						gap: "16px",
						children: { explicitList: ["settings-header", "settings-list"] },
					},
				},
				{
					id: "settings-header",
					component: {
						type: "text" as const,
						content: { literalString: "Notification Settings" },
						variant: "heading" as const,
						size: "lg" as const,
					},
				},
				{
					id: "settings-list",
					component: {
						type: "column" as const,
						gap: "12px",
						children: {
							explicitList: ["email-toggle", "push-toggle", "sms-toggle"],
						},
					},
				},
				{
					id: "email-toggle",
					style: {
						className:
							"flex items-center justify-between p-3 rounded-lg border",
					},
					component: {
						type: "row" as const,
						align: "center" as const,
						justify: "between" as const,
						children: { explicitList: ["email-label", "email-switch"] },
					},
				},
				{
					id: "email-label",
					component: {
						type: "column" as const,
						gap: "2px",
						children: { explicitList: ["email-title", "email-desc"] },
					},
				},
				{
					id: "email-title",
					component: {
						type: "text" as const,
						content: { literalString: "Email Notifications" },
						variant: "body" as const,
						weight: "medium" as const,
					},
				},
				{
					id: "email-desc",
					component: {
						type: "text" as const,
						content: { literalString: "Receive updates via email" },
						variant: "caption" as const,
						color: "text-muted-foreground",
					},
				},
				{
					id: "email-switch",
					component: {
						type: "switch" as const,
						checked: { path: "$.settings.emailEnabled" },
					},
				},
				{
					id: "push-toggle",
					style: {
						className:
							"flex items-center justify-between p-3 rounded-lg border",
					},
					component: {
						type: "row" as const,
						align: "center" as const,
						justify: "between" as const,
						children: { explicitList: ["push-label", "push-switch"] },
					},
				},
				{
					id: "push-label",
					component: {
						type: "column" as const,
						gap: "2px",
						children: { explicitList: ["push-title", "push-desc"] },
					},
				},
				{
					id: "push-title",
					component: {
						type: "text" as const,
						content: { literalString: "Push Notifications" },
						variant: "body" as const,
						weight: "medium" as const,
					},
				},
				{
					id: "push-desc",
					component: {
						type: "text" as const,
						content: { literalString: "Get notified on your device" },
						variant: "caption" as const,
						color: "text-muted-foreground",
					},
				},
				{
					id: "push-switch",
					component: {
						type: "switch" as const,
						checked: { path: "$.settings.pushEnabled" },
					},
				},
				{
					id: "sms-toggle",
					style: {
						className:
							"flex items-center justify-between p-3 rounded-lg border",
					},
					component: {
						type: "row" as const,
						align: "center" as const,
						justify: "between" as const,
						children: { explicitList: ["sms-label", "sms-switch"] },
					},
				},
				{
					id: "sms-label",
					component: {
						type: "column" as const,
						gap: "2px",
						children: { explicitList: ["sms-title", "sms-desc"] },
					},
				},
				{
					id: "sms-title",
					component: {
						type: "text" as const,
						content: { literalString: "SMS Notifications" },
						variant: "body" as const,
						weight: "medium" as const,
					},
				},
				{
					id: "sms-desc",
					component: {
						type: "text" as const,
						content: {
							literalString: "Receive text messages for urgent updates",
						},
						variant: "caption" as const,
						color: "text-muted-foreground",
					},
				},
				{
					id: "sms-switch",
					component: {
						type: "switch" as const,
						checked: { path: "$.settings.smsEnabled" },
					},
				},
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
		const normalizedOutput = {
			...example.output,
			components: validateComponents(
				example.output.components as Parameters<typeof validateComponents>[0],
			).components,
		};
		prompt += `### ${example.name}\nUser: ${example.input}\nAssistant:\n\`\`\`json\n${JSON.stringify(normalizedOutput, null, 2)}\n\`\`\`\n\n`;
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
		const jsonMatch = response.match(/```json\s*([\s\S]*?)```/);
		const jsonStr = jsonMatch ? jsonMatch[1] : response;

		const parsed = JSON.parse(jsonStr.trim());

		if (!parsed.rootComponentId || !Array.isArray(parsed.components)) {
			console.error("Invalid A2UI response structure");
			return null;
		}

		// Apply the same validation/coercion used for AI-emitted surfaces, including converting
		// bare primitive props into BoundValue literals.
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
5. If adding new components, generate appropriate IDs`;
