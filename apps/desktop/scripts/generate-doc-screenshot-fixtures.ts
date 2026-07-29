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
board.page_ids = [];
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
		default_value: Array.from(new TextEncoder().encode("help@acme.example")),
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
		default_value: Array.from(new TextEncoder().encode("Friendly")),
		editable: true,
		exposed: true,
		secret: false,
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
	page_ids: [],
	widget_ids: [],
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
	get_pages: [],
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
	db_table_names: ["customers", "support_tickets", "workflow_runs"],
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
	graph_list_overlays: [],
	graph_list_remote_ontology_imports: [],
	graph_list_imports: [],
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
