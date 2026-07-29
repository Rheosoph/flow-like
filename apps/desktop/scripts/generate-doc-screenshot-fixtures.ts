#!/usr/bin/env bun

import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type JsonRecord = Record<string, unknown>;

const repositoryRoot = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
	"..",
);
const fixturesDirectory = resolve(
	repositoryRoot,
	"apps/desktop/lib/doc-screenshot/fixtures",
);

const formatGeneratedJson = async (paths: string[]): Promise<void> => {
	const formatter = Bun.spawn(
		["bun", "x", "biome", "format", "--write", ...paths],
		{
			cwd: repositoryRoot,
			stdout: "inherit",
			stderr: "inherit",
		},
	);
	const exitCode = await formatter.exited;
	if (exitCode !== 0) {
		throw new Error(`Biome formatting failed with exit code ${exitCode}.`);
	}
};

const readJson = async <T>(path: string): Promise<T> =>
	JSON.parse(await readFile(path, "utf8")) as T;

const timestamp = {
	secs_since_epoch: 1_785_283_200,
	nanos_since_epoch: 0,
};

const encodeJson = (value: unknown): number[] =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

const literalString = (value: string) => ({ literalString: value });
const literalNumber = (value: number) => ({ literalNumber: value });

const onboardingFixture = await readJson<{
	schema: string;
	strict: boolean;
	responses: Record<string, unknown>;
}>(resolve(fixturesDirectory, "onboarding.tauri.json"));
const websiteBoard = await readJson<JsonRecord>(
	resolve(repositoryRoot, "apps/website/src/assets/site.json"),
);
const legacyGraph = await readJson<{
	nodes: Array<{ data?: { node?: JsonRecord } }>;
}>(resolve(repositoryRoot, "apps/docs/src/assets/board.json"));

const eventNode = legacyGraph.nodes
	.map((node) => node.data?.node)
	.find((node) => node?.name === "events_simple");
if (!eventNode) {
	throw new Error("Could not find the events_simple documentation node.");
}

const quickActionNode = structuredClone(eventNode);
quickActionNode.id = "triage-request-node";
quickActionNode.friendly_name = "Quick Action";
quickActionNode.coordinates = [120, 240, 0];

const chatNode = structuredClone(eventNode);
chatNode.id = "support-chat-node";
chatNode.name = "events_chat";
chatNode.friendly_name = "Chat Message";
chatNode.coordinates = [120, 520, 0];

const board = structuredClone(websiteBoard);
board.id = "docs-board";
board.name = "Customer Support Automation";
board.description =
	"Routes incoming questions through a model and prepares a helpful response.";
board.created_at = timestamp;
board.updated_at = timestamp;
board.page_ids = [
	"support-operations-page",
	"customer-intake-page",
	"escalation-console-page",
];
board.nodes = {
	...(board.nodes as JsonRecord),
	[quickActionNode.id as string]: quickActionNode,
	[chatNode.id as string]: chatNode,
};
board.variables = {
	support_email: {
		id: "support-email",
		name: "Support email",
		description: "Mailbox used for escalations",
		data_type: "String",
		value_type: "Normal",
		default_value: encodeJson("help@acme.example"),
		editable: true,
		exposed: true,
		secret: false,
	},
	response_tone: {
		id: "response-tone",
		name: "Response tone",
		description: "Voice used for generated replies",
		data_type: "String",
		value_type: "Normal",
		default_value: encodeJson("Friendly"),
		editable: true,
		exposed: true,
		secret: false,
	},
	crm_api_token: {
		id: "crm-api-token",
		name: "CRM_API_TOKEN",
		description: "Credential used to load customer account context",
		data_type: "String",
		value_type: "Normal",
		default_value: encodeJson(""),
		editable: true,
		exposed: false,
		secret: true,
		runtime_configured: true,
	},
	support_api_url: {
		id: "support-api-url",
		name: "SUPPORT_API_URL",
		description: "Environment-specific endpoint for the support service",
		data_type: "String",
		value_type: "Normal",
		default_value: encodeJson(""),
		editable: true,
		exposed: false,
		secret: false,
		runtime_configured: true,
	},
};

const profile = {
	id: "docs-profile",
	name: "Documentation",
	description: "A deterministic workspace for current product documentation.",
	icon: "/app-logo.webp",
	thumbnail: "/swimlanes/studio.jpg",
	interests: ["automation", "documentation", "AI"],
	tags: ["docs", "examples"],
	hub: "api.flow-like.com",
	secure: true,
	hubs: [],
	apps: [
		{
			app_id: "docs-app",
			favorite: true,
			pinned: true,
			favorite_order: 0,
			pinned_order: 0,
		},
	],
	shortcuts: [],
	theme: null,
	bits: ["docs:gpt-5-mini"],
	custom_bits: [],
	settings: {
		connection_mode: "simplebezier",
	},
	created: "2026-07-20T09:00:00Z",
	updated: "2026-07-28T14:00:00Z",
};

const secondProfile = {
	...profile,
	id: "docs-personal-profile",
	name: "Personal Projects",
	description: "A second profile used to document profile switching.",
	icon: "/flow/icons/computer.svg",
	apps: [],
	bits: [],
};

const settingsProfile = {
	hub_profile: profile,
	execution_settings: {
		gpu_mode: false,
		max_context_size: 128_000,
	},
	created: "2026-07-20T09:00:00Z",
	updated: "2026-07-28T14:00:00Z",
};

const secondSettingsProfile = {
	...settingsProfile,
	hub_profile: secondProfile,
};

const app = {
	id: "docs-app",
	status: "Active",
	visibility: "Offline",
	authors: ["Flow-Like Documentation"],
	bits: [],
	boards: ["docs-board"],
	events: ["triage-selected-request", "support-assistant"],
	templates: [],
	page_ids: [
		"support-operations-page",
		"customer-intake-page",
		"escalation-console-page",
	],
	widget_ids: ["support-health-widget"],
	packages: {},
	rating_sum: 48,
	rating_count: 12,
	avg_rating: 4,
	download_count: 1_284,
	interactions_count: 8_642,
	execution_mode: "Local",
	allow_forking: true,
	version: "1.4.0",
	changelog: "Improved routing and response quality.",
	primary_category: "CustomerSupport",
	secondary_category: "Productivity",
	price: null,
	created_at: timestamp,
	updated_at: timestamp,
};

const metadata = {
	name: "Customer Support Copilot",
	description: "Triage questions and draft consistent, helpful replies.",
	long_description:
		"A complete support workflow with quick actions, a branded chat surface, structured customer data, and human escalation.",
	tags: ["support", "automation", "AI"],
	use_case: "Customer support",
	icon: "/app-logo.webp",
	thumbnail: "/swimlanes/studio.jpg",
	preview_media: [],
	age_rating: null,
	website: "https://flow-like.com",
	support_url: "https://docs.flow-like.com",
	docs_url: "https://docs.flow-like.com",
	release_notes: null,
	organization_specific_values: [],
	created_at: timestamp,
	updated_at: timestamp,
};

const supportDashboardComponents = [
	{
		id: "root",
		style: {
			className: "min-h-full w-full bg-background text-foreground p-6 md:p-8",
		},
		component: {
			type: "column",
			gap: literalString("24px"),
			children: {
				explicitList: ["dashboard-header", "metrics-grid", "operations-grid"],
			},
		},
	},
	{
		id: "dashboard-header",
		style: {
			className: "w-full",
		},
		component: {
			type: "row",
			align: literalString("center"),
			justify: literalString("between"),
			children: {
				explicitList: ["dashboard-heading", "live-status"],
			},
		},
	},
	{
		id: "dashboard-heading",
		component: {
			type: "column",
			gap: literalString("4px"),
			children: {
				explicitList: ["dashboard-title", "dashboard-subtitle"],
			},
		},
	},
	{
		id: "dashboard-title",
		component: {
			type: "text",
			content: literalString("Support operations"),
			variant: literalString("heading"),
			size: literalString("2xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "dashboard-subtitle",
		component: {
			type: "text",
			content: literalString(
				"Queue health, response quality, and SLA risk at a glance",
			),
			variant: literalString("body"),
			size: literalString("sm"),
			color: literalString("muted"),
		},
	},
	{
		id: "live-status",
		component: {
			type: "badge",
			content: literalString("Live · updated now"),
			variant: literalString("secondary"),
		},
	},
	{
		id: "metrics-grid",
		style: {
			className: "w-full grid-cols-1 md:grid-cols-3",
		},
		component: {
			type: "grid",
			columns: literalNumber(3),
			gap: literalString("16px"),
			children: {
				explicitList: ["open-tickets-card", "first-response-card", "sla-card"],
			},
		},
	},
	{
		id: "open-tickets-card",
		style: {
			className: "border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("Open tickets"),
			description: literalString("Across email and chat"),
			variant: literalString("bordered"),
			children: {
				explicitList: ["open-tickets-value", "open-tickets-note"],
			},
		},
	},
	{
		id: "open-tickets-value",
		component: {
			type: "text",
			content: literalString("24"),
			variant: literalString("heading"),
			size: literalString("3xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "open-tickets-note",
		component: {
			type: "text",
			content: literalString("6 need attention"),
			size: literalString("sm"),
			color: literalString("muted"),
		},
	},
	{
		id: "first-response-card",
		style: {
			className: "border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("First response"),
			description: literalString("Median today"),
			variant: literalString("bordered"),
			children: {
				explicitList: ["first-response-value", "first-response-note"],
			},
		},
	},
	{
		id: "first-response-value",
		component: {
			type: "text",
			content: literalString("4m 18s"),
			variant: literalString("heading"),
			size: literalString("3xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "first-response-note",
		component: {
			type: "text",
			content: literalString("12% faster than last week"),
			size: literalString("sm"),
			color: literalString("muted"),
		},
	},
	{
		id: "sla-card",
		style: {
			className: "border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("Within SLA"),
			description: literalString("Rolling 24 hours"),
			variant: literalString("bordered"),
			children: {
				explicitList: ["sla-value", "sla-progress"],
			},
		},
	},
	{
		id: "sla-value",
		component: {
			type: "text",
			content: literalString("94%"),
			variant: literalString("heading"),
			size: literalString("3xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "sla-progress",
		component: {
			type: "progress",
			value: literalNumber(94),
			max: literalNumber(100),
			showLabel: { literalBool: false },
			variant: literalString("success"),
		},
	},
	{
		id: "operations-grid",
		style: {
			className: "w-full grid-cols-1 lg:grid-cols-2",
		},
		component: {
			type: "grid",
			columns: literalNumber(2),
			gap: literalString("16px"),
			children: {
				explicitList: ["priority-queue-card", "automation-card"],
			},
		},
	},
	{
		id: "priority-queue-card",
		style: {
			className: "border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("Priority queue"),
			description: literalString("Tickets closest to their SLA"),
			variant: literalString("bordered"),
			children: {
				explicitList: ["ticket-one", "ticket-two"],
			},
		},
	},
	{
		id: "ticket-one",
		style: {
			className: "w-full rounded-lg border border-border bg-muted/30 p-3",
		},
		component: {
			type: "row",
			align: literalString("center"),
			justify: literalString("between"),
			children: {
				explicitList: ["ticket-one-copy", "ticket-one-status"],
			},
		},
	},
	{
		id: "ticket-one-copy",
		component: {
			type: "text",
			content: literalString("CUS-1042 · Billing access"),
			size: literalString("sm"),
			weight: literalString("medium"),
		},
	},
	{
		id: "ticket-one-status",
		component: {
			type: "badge",
			content: literalString("12 min"),
			variant: literalString("destructive"),
		},
	},
	{
		id: "ticket-two",
		style: {
			className: "w-full rounded-lg border border-border bg-muted/30 p-3",
		},
		component: {
			type: "row",
			align: literalString("center"),
			justify: literalString("between"),
			children: {
				explicitList: ["ticket-two-copy", "ticket-two-status"],
			},
		},
	},
	{
		id: "ticket-two-copy",
		component: {
			type: "text",
			content: literalString("CUS-1027 · Onboarding blocker"),
			size: literalString("sm"),
			weight: literalString("medium"),
		},
	},
	{
		id: "ticket-two-status",
		component: {
			type: "badge",
			content: literalString("28 min"),
			variant: literalString("outline"),
		},
	},
	{
		id: "automation-card",
		style: {
			className: "border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("Automation coverage"),
			description: literalString("Resolution steps handled by Flows"),
			variant: literalString("bordered"),
			children: {
				explicitList: [
					"automation-value",
					"automation-progress",
					"automation-note",
				],
			},
		},
	},
	{
		id: "automation-value",
		component: {
			type: "text",
			content: literalString("68%"),
			variant: literalString("heading"),
			size: literalString("3xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "automation-progress",
		component: {
			type: "progress",
			value: literalNumber(68),
			max: literalNumber(100),
			showLabel: { literalBool: false },
		},
	},
	{
		id: "automation-note",
		component: {
			type: "text",
			content: literalString("184 conversations assisted this week"),
			size: literalString("sm"),
			color: literalString("muted"),
		},
	},
];

const supportPage = {
	id: "support-operations-page",
	name: "Support Operations Dashboard",
	route: "/support",
	title: "Support Operations",
	canvasSettings: {
		backgroundColor: "#09090b",
		padding: "0px",
	},
	content: [],
	layoutType: "Grid",
	components: supportDashboardComponents,
	version: [1, 2, 0],
	createdAt: "2026-07-20T09:00:00Z",
	updatedAt: "2026-07-28T14:00:00Z",
	boardId: "docs-board",
	meta: {
		description: "Live support queue health and SLA risk.",
		keywords: ["support", "operations", "SLA"],
		themeColor: "#7c3aed",
	},
	cache: true,
};

const pageList = [
	{
		appId: "docs-app",
		pageId: supportPage.id,
		boardId: "docs-board",
		name: supportPage.name,
		description: "Live queue health, response quality, and SLA risk.",
	},
	{
		appId: "docs-app",
		pageId: "customer-intake-page",
		boardId: "docs-board",
		name: "Customer Intake",
		description: "Capture account context before a support conversation.",
	},
	{
		appId: "docs-app",
		pageId: "escalation-console-page",
		boardId: "docs-board",
		name: "Escalation Console",
		description: "Review priority cases and hand them to a specialist.",
	},
];

const supportHealthWidgetComponents = [
	{
		id: "root",
		style: {
			className:
				"w-full max-w-lg mx-auto border border-border bg-card shadow-sm",
		},
		component: {
			type: "card",
			title: literalString("Support health"),
			description: literalString("Live performance across customer channels"),
			variant: literalString("bordered"),
			children: {
				explicitList: ["health-summary", "health-progress", "health-footer"],
			},
		},
	},
	{
		id: "health-summary",
		style: {
			className: "w-full",
		},
		component: {
			type: "row",
			align: literalString("center"),
			justify: literalString("between"),
			children: {
				explicitList: ["health-score", "health-status"],
			},
		},
	},
	{
		id: "health-score",
		component: {
			type: "text",
			content: literalString("94%"),
			variant: literalString("heading"),
			size: literalString("3xl"),
			weight: literalString("bold"),
		},
	},
	{
		id: "health-status",
		component: {
			type: "badge",
			content: literalString("On target"),
			variant: literalString("secondary"),
		},
	},
	{
		id: "health-progress",
		component: {
			type: "progress",
			value: literalNumber(94),
			max: literalNumber(100),
			showLabel: { literalBool: false },
			variant: literalString("success"),
		},
	},
	{
		id: "health-footer",
		component: {
			type: "text",
			content: literalString("Median first response · 4m 18s"),
			size: literalString("sm"),
			color: literalString("muted"),
		},
	},
];

const supportHealthWidget = {
	id: "support-health-widget",
	name: "Support Health Card",
	description: "A reusable SLA and response-time summary for support pages.",
	rootComponentId: "root",
	components: supportHealthWidgetComponents,
	dataModel: [
		{ path: "$.health.score", value: 94 },
		{ path: "$.health.responseTime", value: "4m 18s" },
	],
	customizationOptions: [],
	exposedProps: [
		{
			id: "title",
			label: "Card title",
			description: "Heading shown above the support metrics.",
			targetComponentId: "root",
			propertyPath: "component.title",
			propType: "String",
			defaultValue: encodeJson("Support health"),
			group: "Content",
		},
	],
	tags: ["support", "metrics", "SLA"],
	version: [1, 1, 0],
	createdAt: "2026-07-20T09:00:00Z",
	updatedAt: "2026-07-28T14:00:00Z",
	actions: [
		{
			id: "open-priority-queue",
			label: "Open priority queue",
			description: "Navigate to the queue that needs attention.",
			contextSchema: [],
		},
	],
};

const widgetMetadata = {
	name: supportHealthWidget.name,
	description: supportHealthWidget.description,
	long_description:
		"Use this widget on dashboards that need a compact view of SLA attainment and response speed.",
	tags: supportHealthWidget.tags,
	icon: null,
	thumbnail: null,
	preview_media: [],
	age_rating: null,
	website: null,
	support_url: null,
	docs_url: null,
	release_notes: null,
	organization_specific_values: [],
	use_case: "Customer support",
	created_at: timestamp,
	updated_at: timestamp,
};

const quickActionEvent = {
	id: "triage-selected-request",
	name: "Triage selected request",
	description: "Analyze the selected customer request and prepare a response.",
	board_id: "docs-board",
	board_version: null,
	node_id: "triage-request-node",
	variables: {},
	inputs: [],
	config: encodeJson({
		navigate_to_routes: ["/chat"],
	}),
	active: true,
	priority: 10,
	event_type: "quick_action",
	event_version: [1, 1, 0],
	execution_mode: "Local",
	exposure: "INTERNAL",
	route: null,
	is_default: false,
	default_page_id: null,
	canary: null,
	correlation_mappings: [],
	notes: "Available from the app quick-action menu.",
	created_at: timestamp,
	updated_at: timestamp,
};

const chatEvent = {
	...quickActionEvent,
	id: "support-assistant",
	name: "Support assistant",
	description: "A welcoming chat interface for customer questions.",
	node_id: "support-chat-node",
	config: encodeJson({
		allow_file_upload: true,
		allow_voice_input: false,
		ai_disclosure: "AI assistant — responses may need human review.",
		background_image: "",
		custom_css: "",
		history_elements: 10,
		tools: [],
		default_tools: [],
		example_messages: [
			"Where is my order?",
			"Help me update my subscription",
			"I need to speak with support",
		],
		color_scheme: "dark",
	}),
	priority: 20,
	event_type: "simple_chat",
	event_version: [2, 0, 0],
	route: "/chat",
	is_default: true,
	notes: "Primary customer-facing chat experience.",
};

const fixedGraphStyle = (
	color: string,
	icon: string,
	value = 14,
): JsonRecord => ({
	color,
	icon,
	size: { mode: "fixed", value },
	shape: "circle",
});

const customerOperationsOntology = {
	id: "customer-operations",
	name: "Customer Operations",
	description:
		"A live semantic model connecting customers, accounts, support work, products, and knowledge.",
	nodes: [
		{
			id: "customer",
			api_name: "customer",
			label: "Customer",
			table: "customers",
			id_column: "customer_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "plan", data_type: "Utf8", nullable: false },
				{ name: "status", data_type: "Utf8", nullable: false },
				{ name: "region", data_type: "Utf8", nullable: false },
				{ name: "open_tickets", data_type: "Int64", nullable: false },
			],
			style: fixedGraphStyle("#14b8a6", "user", 16),
		},
		{
			id: "account",
			api_name: "account",
			label: "Account",
			table: "accounts",
			id_column: "account_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "segment", data_type: "Utf8", nullable: false },
				{ name: "arr", data_type: "Float64", nullable: false },
				{ name: "health", data_type: "Utf8", nullable: false },
			],
			style: fixedGraphStyle("#6366f1", "briefcase", 17),
		},
		{
			id: "ticket",
			api_name: "support_ticket",
			label: "Support Ticket",
			table: "support_tickets",
			id_column: "ticket_id",
			display_column: "subject",
			property_columns: [
				{ name: "subject", data_type: "Utf8", nullable: false },
				{ name: "priority", data_type: "Utf8", nullable: false },
				{ name: "status", data_type: "Utf8", nullable: false },
				{ name: "channel", data_type: "Utf8", nullable: false },
				{ name: "sla_minutes", data_type: "Int64", nullable: false },
			],
			style: {
				color: "#f59e0b",
				icon: "alertTriangle",
				size: { mode: "by-degree", min: 13, max: 24 },
				shape: "circle",
			},
		},
		{
			id: "agent",
			api_name: "support_agent",
			label: "Support Agent",
			table: "support_agents",
			id_column: "agent_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "team", data_type: "Utf8", nullable: false },
				{ name: "timezone", data_type: "Utf8", nullable: false },
				{ name: "capacity", data_type: "Int64", nullable: false },
			],
			style: fixedGraphStyle("#ec4899", "userCog", 15),
		},
		{
			id: "product",
			api_name: "product",
			label: "Product",
			table: "products",
			id_column: "product_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "area", data_type: "Utf8", nullable: false },
				{ name: "owner", data_type: "Utf8", nullable: false },
			],
			style: fixedGraphStyle("#22c55e", "package", 16),
		},
		{
			id: "article",
			api_name: "knowledge_article",
			label: "Knowledge Article",
			table: "knowledge_articles",
			id_column: "article_id",
			display_column: "title",
			property_columns: [
				{ name: "title", data_type: "Utf8", nullable: false },
				{ name: "collection", data_type: "Utf8", nullable: false },
				{ name: "helpfulness", data_type: "Float64", nullable: false },
			],
			style: fixedGraphStyle("#8b5cf6", "fileText", 14),
		},
	],
	edges: [
		{
			id: "customer-account",
			api_name: "belongs_to_account",
			label: "belongs to",
			table: "customer_accounts",
			src_column: "customer_id",
			dst_column: "account_id",
			src_label: "Customer",
			dst_label: "Account",
			property_columns: [],
			style: {
				...fixedGraphStyle("#818cf8", "link", 10),
				width: 1.8,
			},
		},
		{
			id: "customer-ticket",
			api_name: "opened_ticket",
			label: "opened",
			table: "support_tickets",
			src_column: "customer_id",
			dst_column: "ticket_id",
			src_label: "Customer",
			dst_label: "Support Ticket",
			containment: true,
			property_columns: [
				{ name: "created_at", data_type: "Timestamp", nullable: false },
			],
			style: {
				...fixedGraphStyle("#2dd4bf", "link", 10),
				width: 2,
			},
		},
		{
			id: "ticket-agent",
			api_name: "assigned_to_agent",
			label: "assigned to",
			table: "ticket_assignments",
			src_column: "ticket_id",
			dst_column: "agent_id",
			src_label: "Support Ticket",
			dst_label: "Support Agent",
			property_columns: [
				{ name: "assigned_at", data_type: "Timestamp", nullable: false },
			],
			style: {
				...fixedGraphStyle("#f472b6", "link", 10),
				width: 1.8,
			},
		},
		{
			id: "ticket-product",
			api_name: "concerns_product",
			label: "concerns",
			table: "ticket_products",
			src_column: "ticket_id",
			dst_column: "product_id",
			src_label: "Support Ticket",
			dst_label: "Product",
			property_columns: [],
			style: {
				...fixedGraphStyle("#4ade80", "link", 10),
				width: 1.6,
			},
		},
		{
			id: "ticket-article",
			api_name: "suggests_article",
			label: "suggests",
			table: "ticket_articles",
			src_column: "ticket_id",
			dst_column: "article_id",
			src_label: "Support Ticket",
			dst_label: "Knowledge Article",
			property_columns: [
				{ name: "confidence", data_type: "Float64", nullable: false },
			],
			style: {
				...fixedGraphStyle("#a78bfa", "link", 10),
				width: 1.5,
			},
		},
		{
			id: "account-product",
			api_name: "uses_product",
			label: "uses",
			table: "account_products",
			src_column: "account_id",
			dst_column: "product_id",
			src_label: "Account",
			dst_label: "Product",
			property_columns: [
				{ name: "adoption", data_type: "Utf8", nullable: false },
			],
			style: {
				...fixedGraphStyle("#60a5fa", "link", 10),
				width: 1.4,
			},
		},
	],
	object_views: [
		{
			object_type: "customer",
			title_property: "name",
			prominent_properties: ["plan", "status", "region", "open_tickets"],
		},
		{
			object_type: "ticket",
			title_property: "subject",
			prominent_properties: ["priority", "status", "channel", "sla_minutes"],
		},
	],
	actions: [
		{
			id: "triage-ticket",
			name: "Triage support ticket",
			description:
				"Classify urgency, find relevant context, and prepare the next best response.",
			object_type: "ticket",
			board_id: "docs-board",
			board_version: [1, 2, 0],
			start_node_id: "triage-request-node",
			event_id: "triage-selected-request",
			enabled: true,
			allow_bulk: true,
			exposed: true,
			parameter_schema: {
				type: "object",
				properties: {
					response_tone: {
						type: "string",
						title: "Response tone",
						default: "Friendly",
					},
				},
			},
		},
		{
			id: "refresh-account-brief",
			name: "Refresh account brief",
			description:
				"Summarize account health, recent requests, and product adoption.",
			object_type: "account",
			board_id: "docs-board",
			board_version: [1, 2, 0],
			start_node_id: "triage-request-node",
			enabled: true,
			allow_bulk: false,
			exposed: false,
		},
	],
	exposed: true,
	bindings_enabled: true,
	default_limit: 200,
	created_at: "2026-07-20T09:00:00Z",
	updated_at: "2026-07-28T14:00:00Z",
};

const productKnowledgeOntology = {
	id: "product-knowledge",
	name: "Product Knowledge",
	description:
		"Published product areas, capabilities, and support guidance from the Knowledge Hub.",
	nodes: [
		{
			id: "capability",
			api_name: "capability",
			label: "Capability",
			table: "capabilities",
			id_column: "capability_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "maturity", data_type: "Utf8", nullable: false },
			],
			style: fixedGraphStyle("#38bdf8", "package", 15),
		},
		{
			id: "guide",
			api_name: "guide",
			label: "Guide",
			table: "guides",
			id_column: "guide_id",
			display_column: "title",
			property_columns: [
				{ name: "title", data_type: "Utf8", nullable: false },
				{ name: "audience", data_type: "Utf8", nullable: false },
			],
			style: fixedGraphStyle("#a78bfa", "fileText", 14),
		},
		{
			id: "owner",
			api_name: "product_owner",
			label: "Product Owner",
			table: "product_owners",
			id_column: "owner_id",
			display_column: "name",
			property_columns: [
				{ name: "name", data_type: "Utf8", nullable: false },
				{ name: "team", data_type: "Utf8", nullable: false },
			],
			style: fixedGraphStyle("#fb7185", "userCog", 14),
		},
	],
	edges: [
		{
			id: "capability-guide",
			api_name: "documented_by",
			label: "documented by",
			table: "capability_guides",
			src_column: "capability_id",
			dst_column: "guide_id",
			src_label: "Capability",
			dst_label: "Guide",
			containment: true,
			property_columns: [],
			style: {
				...fixedGraphStyle("#a78bfa", "link", 10),
				width: 1.5,
			},
		},
		{
			id: "capability-owner",
			api_name: "owned_by",
			label: "owned by",
			table: "capability_owners",
			src_column: "capability_id",
			dst_column: "owner_id",
			src_label: "Capability",
			dst_label: "Product Owner",
			property_columns: [],
			style: {
				...fixedGraphStyle("#fb7185", "link", 10),
				width: 1.5,
			},
		},
	],
	object_views: [],
	actions: [],
	exposed: true,
	bindings_enabled: true,
	default_limit: 100,
	created_at: "2026-07-18T08:30:00Z",
	updated_at: "2026-07-27T16:15:00Z",
};

const installedProductKnowledgeOntology = {
	id: "remote-product-knowledge",
	target_app_id: "knowledge-hub",
	remote_ontology_id: productKnowledgeOntology.id,
	contract: productKnowledgeOntology,
	source_updated_at: productKnowledgeOntology.updated_at,
	bindings_enabled: true,
	installed_at: "2026-07-27T16:20:00Z",
	updated_at: "2026-07-27T16:20:00Z",
};

const ontologyCustomerRows = [
	{
		customer_id: "CUS-1042",
		name: "Avery Morgan",
		plan: "Enterprise",
		status: "Active",
		region: "North America",
		open_tickets: 1,
	},
	{
		customer_id: "CUS-1038",
		name: "Jordan Lee",
		plan: "Team",
		status: "Active",
		region: "Europe",
		open_tickets: 0,
	},
	{
		customer_id: "CUS-1027",
		name: "Samira Patel",
		plan: "Enterprise",
		status: "Onboarding",
		region: "Asia Pacific",
		open_tickets: 2,
	},
	{
		customer_id: "CUS-1019",
		name: "Noah Williams",
		plan: "Starter",
		status: "Trial",
		region: "North America",
		open_tickets: 1,
	},
	{
		customer_id: "CUS-1004",
		name: "Mina Chen",
		plan: "Team",
		status: "Active",
		region: "Europe",
		open_tickets: 0,
	},
];

const graphNode = (
	label: string,
	id: string,
	caption: string,
	props: JsonRecord,
) => ({
	id: `${label}:${id}`,
	label,
	caption,
	props,
});
const graphEdge = (
	id: string,
	source: string,
	target: string,
	label: string,
	props: JsonRecord = {},
) => ({ id, source, target, label, props });

const customerOperationsSubgraphFixture = {
	nodes: [
		...ontologyCustomerRows.map((customer) =>
			graphNode("Customer", customer.customer_id, customer.name, customer),
		),
		graphNode("Account", "ACC-88", "Northstar Labs", {
			account_id: "ACC-88",
			name: "Northstar Labs",
			segment: "Enterprise",
			arr: 248_000,
			health: "Healthy",
		}),
		graphNode("Account", "ACC-74", "Atlas Logistics", {
			account_id: "ACC-74",
			name: "Atlas Logistics",
			segment: "Enterprise",
			arr: 184_000,
			health: "At risk",
		}),
		graphNode("Account", "ACC-61", "Meridian Works", {
			account_id: "ACC-61",
			name: "Meridian Works",
			segment: "Growth",
			arr: 92_000,
			health: "Healthy",
		}),
		graphNode("Account", "ACC-46", "Juniper Studio", {
			account_id: "ACC-46",
			name: "Juniper Studio",
			segment: "Growth",
			arr: 68_000,
			health: "Onboarding",
		}),
		graphNode("Support Ticket", "TKT-2841", "Billing access issue", {
			ticket_id: "TKT-2841",
			subject: "Billing access issue",
			priority: "Urgent",
			status: "In progress",
			channel: "Chat",
			sla_minutes: 12,
		}),
		graphNode("Support Ticket", "TKT-2836", "SSO configuration", {
			ticket_id: "TKT-2836",
			subject: "SSO configuration",
			priority: "High",
			status: "Open",
			channel: "Email",
			sla_minutes: 48,
		}),
		graphNode("Support Ticket", "TKT-2829", "Onboarding blocker", {
			ticket_id: "TKT-2829",
			subject: "Onboarding blocker",
			priority: "High",
			status: "In progress",
			channel: "Chat",
			sla_minutes: 35,
		}),
		graphNode("Support Ticket", "TKT-2822", "Usage report mismatch", {
			ticket_id: "TKT-2822",
			subject: "Usage report mismatch",
			priority: "Normal",
			status: "Waiting",
			channel: "Portal",
			sla_minutes: 120,
		}),
		graphNode("Support Ticket", "TKT-2818", "Invite delivery delayed", {
			ticket_id: "TKT-2818",
			subject: "Invite delivery delayed",
			priority: "Normal",
			status: "Open",
			channel: "Email",
			sla_minutes: 90,
		}),
		graphNode("Support Ticket", "TKT-2811", "API rate limit question", {
			ticket_id: "TKT-2811",
			subject: "API rate limit question",
			priority: "Low",
			status: "Solved",
			channel: "Portal",
			sla_minutes: 240,
		}),
		graphNode("Support Ticket", "TKT-2805", "Workspace migration", {
			ticket_id: "TKT-2805",
			subject: "Workspace migration",
			priority: "Normal",
			status: "In progress",
			channel: "Email",
			sla_minutes: 75,
		}),
		graphNode("Support Agent", "AGT-12", "Nina Torres", {
			agent_id: "AGT-12",
			name: "Nina Torres",
			team: "Enterprise Support",
			timezone: "UTC-5",
			capacity: 72,
		}),
		graphNode("Support Agent", "AGT-18", "Omar Hassan", {
			agent_id: "AGT-18",
			name: "Omar Hassan",
			team: "Technical Support",
			timezone: "UTC+1",
			capacity: 58,
		}),
		graphNode("Support Agent", "AGT-24", "Keiko Sato", {
			agent_id: "AGT-24",
			name: "Keiko Sato",
			team: "Onboarding",
			timezone: "UTC+9",
			capacity: 64,
		}),
		graphNode("Support Agent", "AGT-31", "Lucas Meyer", {
			agent_id: "AGT-31",
			name: "Lucas Meyer",
			team: "Growth Support",
			timezone: "UTC+1",
			capacity: 81,
		}),
		graphNode("Product", "PRD-CORE", "Flow Builder", {
			product_id: "PRD-CORE",
			name: "Flow Builder",
			area: "Automation",
			owner: "Core Experience",
		}),
		graphNode("Product", "PRD-DATA", "Data Studio", {
			product_id: "PRD-DATA",
			name: "Data Studio",
			area: "Data",
			owner: "Knowledge Systems",
		}),
		graphNode("Product", "PRD-IDENTITY", "Identity & Access", {
			product_id: "PRD-IDENTITY",
			name: "Identity & Access",
			area: "Platform",
			owner: "Trust",
		}),
		graphNode("Knowledge Article", "KB-117", "Configure enterprise SSO", {
			article_id: "KB-117",
			title: "Configure enterprise SSO",
			collection: "Administration",
			helpfulness: 0.94,
		}),
		graphNode("Knowledge Article", "KB-204", "Understand usage and billing", {
			article_id: "KB-204",
			title: "Understand usage and billing",
			collection: "Billing",
			helpfulness: 0.91,
		}),
		graphNode("Knowledge Article", "KB-318", "Move an existing workspace", {
			article_id: "KB-318",
			title: "Move an existing workspace",
			collection: "Migration",
			helpfulness: 0.88,
		}),
	],
	edges: [
		graphEdge(
			"e-customer-account-1",
			"Customer:CUS-1042",
			"Account:ACC-88",
			"belongs to",
		),
		graphEdge(
			"e-customer-account-2",
			"Customer:CUS-1038",
			"Account:ACC-61",
			"belongs to",
		),
		graphEdge(
			"e-customer-account-3",
			"Customer:CUS-1027",
			"Account:ACC-74",
			"belongs to",
		),
		graphEdge(
			"e-customer-account-4",
			"Customer:CUS-1019",
			"Account:ACC-46",
			"belongs to",
		),
		graphEdge(
			"e-customer-account-5",
			"Customer:CUS-1004",
			"Account:ACC-61",
			"belongs to",
		),
		graphEdge(
			"e-opened-1",
			"Customer:CUS-1042",
			"Support Ticket:TKT-2841",
			"opened",
		),
		graphEdge(
			"e-opened-2",
			"Customer:CUS-1042",
			"Support Ticket:TKT-2836",
			"opened",
		),
		graphEdge(
			"e-opened-3",
			"Customer:CUS-1027",
			"Support Ticket:TKT-2829",
			"opened",
		),
		graphEdge(
			"e-opened-4",
			"Customer:CUS-1038",
			"Support Ticket:TKT-2822",
			"opened",
		),
		graphEdge(
			"e-opened-5",
			"Customer:CUS-1019",
			"Support Ticket:TKT-2818",
			"opened",
		),
		graphEdge(
			"e-opened-6",
			"Customer:CUS-1004",
			"Support Ticket:TKT-2811",
			"opened",
		),
		graphEdge(
			"e-opened-7",
			"Customer:CUS-1027",
			"Support Ticket:TKT-2805",
			"opened",
		),
		graphEdge(
			"e-assigned-1",
			"Support Ticket:TKT-2841",
			"Support Agent:AGT-12",
			"assigned to",
		),
		graphEdge(
			"e-assigned-2",
			"Support Ticket:TKT-2836",
			"Support Agent:AGT-18",
			"assigned to",
		),
		graphEdge(
			"e-assigned-3",
			"Support Ticket:TKT-2829",
			"Support Agent:AGT-24",
			"assigned to",
		),
		graphEdge(
			"e-assigned-4",
			"Support Ticket:TKT-2822",
			"Support Agent:AGT-31",
			"assigned to",
		),
		graphEdge(
			"e-assigned-5",
			"Support Ticket:TKT-2818",
			"Support Agent:AGT-18",
			"assigned to",
		),
		graphEdge(
			"e-assigned-6",
			"Support Ticket:TKT-2811",
			"Support Agent:AGT-31",
			"assigned to",
		),
		graphEdge(
			"e-assigned-7",
			"Support Ticket:TKT-2805",
			"Support Agent:AGT-24",
			"assigned to",
		),
		graphEdge(
			"e-product-1",
			"Support Ticket:TKT-2841",
			"Product:PRD-DATA",
			"concerns",
		),
		graphEdge(
			"e-product-2",
			"Support Ticket:TKT-2836",
			"Product:PRD-IDENTITY",
			"concerns",
		),
		graphEdge(
			"e-product-3",
			"Support Ticket:TKT-2829",
			"Product:PRD-CORE",
			"concerns",
		),
		graphEdge(
			"e-product-4",
			"Support Ticket:TKT-2822",
			"Product:PRD-DATA",
			"concerns",
		),
		graphEdge(
			"e-product-5",
			"Support Ticket:TKT-2818",
			"Product:PRD-IDENTITY",
			"concerns",
		),
		graphEdge(
			"e-product-6",
			"Support Ticket:TKT-2811",
			"Product:PRD-CORE",
			"concerns",
		),
		graphEdge(
			"e-product-7",
			"Support Ticket:TKT-2805",
			"Product:PRD-CORE",
			"concerns",
		),
		graphEdge(
			"e-article-1",
			"Support Ticket:TKT-2841",
			"Knowledge Article:KB-204",
			"suggests",
			{ confidence: 0.93 },
		),
		graphEdge(
			"e-article-2",
			"Support Ticket:TKT-2836",
			"Knowledge Article:KB-117",
			"suggests",
			{ confidence: 0.96 },
		),
		graphEdge(
			"e-article-3",
			"Support Ticket:TKT-2822",
			"Knowledge Article:KB-204",
			"suggests",
			{ confidence: 0.87 },
		),
		graphEdge(
			"e-article-4",
			"Support Ticket:TKT-2805",
			"Knowledge Article:KB-318",
			"suggests",
			{ confidence: 0.9 },
		),
		graphEdge(
			"e-account-product-1",
			"Account:ACC-88",
			"Product:PRD-DATA",
			"uses",
		),
		graphEdge(
			"e-account-product-2",
			"Account:ACC-74",
			"Product:PRD-CORE",
			"uses",
		),
		graphEdge(
			"e-account-product-3",
			"Account:ACC-61",
			"Product:PRD-DATA",
			"uses",
		),
		graphEdge(
			"e-account-product-4",
			"Account:ACC-46",
			"Product:PRD-IDENTITY",
			"uses",
		),
	],
	truncated: false,
};

const docsGraphNodeIds = new Set([
	"Customer:CUS-1042",
	"Customer:CUS-1038",
	"Customer:CUS-1027",
	"Customer:CUS-1004",
	"Account:ACC-88",
	"Account:ACC-74",
	"Account:ACC-61",
	"Support Ticket:TKT-2841",
	"Support Ticket:TKT-2836",
	"Support Ticket:TKT-2829",
	"Support Ticket:TKT-2822",
	"Support Agent:AGT-12",
	"Support Agent:AGT-18",
	"Support Agent:AGT-24",
	"Support Agent:AGT-31",
	"Product:PRD-CORE",
	"Product:PRD-DATA",
	"Product:PRD-IDENTITY",
	"Knowledge Article:KB-204",
]);

const customerOperationsSubgraph = {
	nodes: customerOperationsSubgraphFixture.nodes.filter((node) =>
		docsGraphNodeIds.has(node.id),
	),
	edges: customerOperationsSubgraphFixture.edges.filter(
		(edge) =>
			docsGraphNodeIds.has(edge.source) && docsGraphNodeIds.has(edge.target),
	),
	truncated: false,
};

const baseResponses: Record<string, unknown> = {
	...onboardingFixture.responses,
	get_current_profile: settingsProfile,
	get_current_profile_id: profile.id,
	get_profiles: {
		[profile.id]: settingsProfile,
		[secondProfile.id]: secondSettingsProfile,
	},
	get_profiles_raw: {
		[profile.id]: settingsProfile,
		[secondProfile.id]: secondSettingsProfile,
	},
	get_apps: [],
	get_app: app,
	get_app_meta: metadata,
	get_app_boards: [board],
	get_board: board,
	get_catalog: Object.values(board.nodes as JsonRecord),
	get_events: [quickActionEvent, chatEvent],
	get_event: quickActionEvent,
	get_app_routes: [
		{ path: "/chat", eventId: "support-assistant" },
		{ path: "/triage", eventId: "triage-selected-request" },
	],
	get_pages: pageList,
	get_page: supportPage,
	get_open_pages: [],
	get_widgets: [supportHealthWidget],
	get_widget: supportHealthWidget,
	get_widget_meta: widgetMetadata,
	get_widget_versions: [
		[1, 0, 0],
		[1, 1, 0],
	],
	get_open_widgets: [],
	storage_list: [
		{
			location: "customer-briefs",
			last_modified: "2026-07-28T13:52:00Z",
			size: 0,
			is_dir: true,
			e_tag: null,
			version: null,
		},
		{
			location: "archived-tickets",
			last_modified: "2026-07-28T12:36:00Z",
			size: 0,
			is_dir: true,
			e_tag: null,
			version: null,
		},
		{
			location: "support-playbook.pdf",
			last_modified: "2026-07-28T11:18:00Z",
			size: 2_846_720,
			is_dir: false,
			e_tag: "docs-playbook-v3",
			version: "3",
		},
		{
			location: "brand-voice.md",
			last_modified: "2026-07-27T16:40:00Z",
			size: 18_432,
			is_dir: false,
			e_tag: "docs-brand-v2",
			version: "2",
		},
		{
			location: "refund-policy.md",
			last_modified: "2026-07-26T09:20:00Z",
			size: 12_288,
			is_dir: false,
			e_tag: "docs-refund-v1",
			version: "1",
		},
	],
	db_table_names: [
		"accounts",
		"customer_accounts",
		"customers",
		"knowledge_articles",
		"products",
		"support_agents",
		"support_tickets",
		"ticket_articles",
		"ticket_assignments",
		"ticket_products",
		"workflow_runs",
	],
	db_table_names_user: [],
	db_schema: {
		metadata: { name: "customers" },
		fields: [
			{ name: "customer_id", data_type: "Utf8", nullable: false },
			{ name: "name", data_type: "Utf8", nullable: false },
			{ name: "plan", data_type: "Utf8", nullable: false },
			{ name: "status", data_type: "Utf8", nullable: false },
			{ name: "open_tickets", data_type: "Int64", nullable: false },
			{ name: "last_contact", data_type: "Timestamp", nullable: true },
		],
	},
	db_count: 5,
	db_list: [
		{
			customer_id: "CUS-1042",
			name: "Avery Morgan",
			plan: "Enterprise",
			status: "Active",
			open_tickets: 1,
			last_contact: "2026-07-28T12:45:00Z",
		},
		{
			customer_id: "CUS-1038",
			name: "Jordan Lee",
			plan: "Team",
			status: "Active",
			open_tickets: 0,
			last_contact: "2026-07-28T09:12:00Z",
		},
		{
			customer_id: "CUS-1027",
			name: "Samira Patel",
			plan: "Enterprise",
			status: "Onboarding",
			open_tickets: 2,
			last_contact: "2026-07-27T17:30:00Z",
		},
		{
			customer_id: "CUS-1019",
			name: "Noah Williams",
			plan: "Starter",
			status: "Trial",
			open_tickets: 1,
			last_contact: "2026-07-26T15:05:00Z",
		},
		{
			customer_id: "CUS-1004",
			name: "Mina Chen",
			plan: "Team",
			status: "Active",
			open_tickets: 0,
			last_contact: "2026-07-25T08:40:00Z",
		},
	],
	db_indices: [],
	graph_list_overlays: [customerOperationsOntology],
	graph_get_overlay: customerOperationsOntology,
	graph_subgraph: customerOperationsSubgraph,
	graph_sample: ontologyCustomerRows,
	graph_search_nodes: [],
	graph_neighbors: customerOperationsSubgraph,
	graph_overlay_children: customerOperationsSubgraph,
	graph_paths: {
		found: false,
		paths: [],
		nodes: [],
		edges: [],
		truncated: false,
	},
	graph_cypher: [],
	graph_list_remote_ontology_imports: [installedProductKnowledgeOntology],
	graph_list_imports: [installedProductKnowledgeOntology],
	flowpilot_list_board_edit_jobs: [],
	list_runs: [],
	query_run: [],
	get_board_versions: [
		[1, 0, 0],
		[1, 1, 0],
		[1, 2, 0],
	],
};

const makeHttpFixture = (
	responses: Record<string, unknown>,
	body: unknown,
) => ({
	schema: onboardingFixture.schema,
	strict: false,
	responses: {
		...responses,
		"plugin:http|fetch": 1,
		"plugin:http|fetch_send": {
			status: 200,
			statusText: "OK",
			url: "https://api.flow-like.com/api/v1/docs",
			headers: [["content-type", "application/json"]],
			rid: 2,
		},
		"plugin:http|fetch_read_body": [
			...Array.from(new TextEncoder().encode(JSON.stringify(body))),
			1,
		],
		"plugin:http|fetch_cancel": null,
		"plugin:http|fetch_cancel_body": null,
	},
});

const fixtures = {
	"docs-apps.tauri.json": {
		schema: onboardingFixture.schema,
		strict: false,
		responses: baseResponses,
	},
	"docs-sharing.tauri.json": makeHttpFixture(
		{
			...baseResponses,
			get_app: { ...app, visibility: "Prototype" },
		},
		[],
	),
	"docs-roles.tauri.json": makeHttpFixture(
		{
			...baseResponses,
			get_app: { ...app, visibility: "Prototype" },
		},
		[null, []],
	),
};

const fixtureEntries = Object.entries(fixtures);
await Promise.all(
	fixtureEntries.map(([name, value]) =>
		writeFile(
			resolve(fixturesDirectory, name),
			`${JSON.stringify(value, null, "\t")}\n`,
			"utf8",
		),
	),
);
await formatGeneratedJson(
	fixtureEntries.map(([name]) => resolve(fixturesDirectory, name)),
);

console.log(
	`Generated ${Object.keys(fixtures).length} documentation fixtures.`,
);
