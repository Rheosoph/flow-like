#!/usr/bin/env bun

import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type JsonRecord = Record<string, unknown>;

type PinOptions = {
	id: string;
	name: string;
	friendlyName: string;
	pinType: "Input" | "Output";
	dataType: string;
	index: number;
	dependsOn?: string[];
	connectedTo?: string[];
	defaultValue?: number[] | null;
	valueType?: "Normal" | "Array" | "HashSet" | "HashMap";
};

type NodeOptions = {
	id: string;
	name: string;
	friendlyName: string;
	description: string;
	category: string;
	coordinates: [number, number, number];
	pins: JsonRecord;
	start?: boolean;
	layer?: string | null;
	icon?: string | null;
	hash: number;
};

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

const formatGeneratedJson = async (path: string): Promise<void> => {
	const formatter = Bun.spawn(
		["bun", "x", "biome", "format", "--write", path],
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

const encodeJson = (value: unknown): number[] =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

const makePin = ({
	id,
	name,
	friendlyName,
	pinType,
	dataType,
	index,
	dependsOn = [],
	connectedTo = [],
	defaultValue = null,
	valueType = "Normal",
}: PinOptions): JsonRecord => ({
	id,
	name,
	friendly_name: friendlyName,
	description: "",
	pin_type: pinType,
	data_type: dataType,
	schema: null,
	value_type: valueType,
	depends_on: dependsOn,
	connected_to: connectedTo,
	default_value: defaultValue,
	index,
	options: null,
});

const makeNode = ({
	id,
	name,
	friendlyName,
	description,
	category,
	coordinates,
	pins,
	start = false,
	layer = null,
	icon = null,
	hash,
}: NodeOptions): JsonRecord => ({
	id,
	name,
	friendly_name: friendlyName,
	description,
	coordinates,
	category,
	scores: null,
	pins,
	start,
	icon,
	comment: null,
	long_running: null,
	error: null,
	docs: null,
	event_callback: null,
	layer,
	hash,
});

const onboardingFixture = await readJson<{
	schema: string;
	strict: boolean;
	responses: Record<string, unknown>;
}>(resolve(fixturesDirectory, "onboarding.tauri.json"));
const websiteBoard = await readJson<JsonRecord>(
	resolve(repositoryRoot, "apps/website/src/assets/site.json"),
);

const eventNode = makeNode({
	id: "docs-event-node",
	name: "events_simple",
	friendlyName: "Incoming Support Request",
	description: "Starts the workflow when a new support request arrives.",
	category: "Events",
	icon: "/flow/icons/workflow.svg",
	coordinates: [820, 55, 0],
	start: true,
	hash: 10_001,
	pins: {
		"docs-event-exec-out": makePin({
			id: "docs-event-exec-out",
			name: "exec_out",
			friendlyName: "Start",
			pinType: "Output",
			dataType: "Execution",
			index: 1,
			connectedTo: ["docs-layer-in"],
		}),
		"docs-event-request-out": makePin({
			id: "docs-event-request-out",
			name: "request",
			friendlyName: "Request",
			pinType: "Output",
			dataType: "String",
			index: 2,
			connectedTo: ["docs-layer-message-in"],
			defaultValue: encodeJson("Where is my order?"),
		}),
	},
});

const stringSourceNode = makeNode({
	id: "docs-string-source",
	name: "string_template",
	friendlyName: "Customer Message",
	description: "Produces a typed String value for the generic input.",
	category: "Text",
	coordinates: [970, 140, 0],
	hash: 10_002,
	pins: {
		"docs-string-out": makePin({
			id: "docs-string-out",
			name: "value",
			friendlyName: "Message",
			pinType: "Output",
			dataType: "String",
			index: 1,
			connectedTo: ["docs-generic-in"],
			defaultValue: encodeJson("Summarize the customer request"),
		}),
		"docs-string-draft-out": makePin({
			id: "docs-string-draft-out",
			name: "draft",
			friendlyName: "Draft",
			pinType: "Output",
			dataType: "String",
			index: 2,
			defaultValue: encodeJson("Draft a customer reply"),
		}),
	},
});

const genericTargetNode = makeNode({
	id: "docs-generic-target",
	name: "transform_generic",
	friendlyName: "Format Generic Value",
	description: "Accepts a generically typed value and returns it unchanged.",
	category: "Transformation",
	coordinates: [1260, 140, 0],
	hash: 10_003,
	pins: {
		"docs-generic-in": makePin({
			id: "docs-generic-in",
			name: "value",
			friendlyName: "Generic Value",
			pinType: "Input",
			dataType: "Generic",
			index: 1,
			dependsOn: ["docs-string-out"],
		}),
		"docs-generic-out": makePin({
			id: "docs-generic-out",
			name: "value_out",
			friendlyName: "Value",
			pinType: "Output",
			dataType: "Generic",
			index: 1,
		}),
	},
});

const layerChildNode = makeNode({
	id: "docs-layer-child",
	name: "text_normalize",
	friendlyName: "Normalize Request",
	description: "Cleans and normalizes the incoming customer request.",
	category: "Text",
	coordinates: [235, 180, 0],
	layer: "docs-layer",
	hash: 10_004,
	pins: {
		"docs-layer-child-in": makePin({
			id: "docs-layer-child-in",
			name: "exec_in",
			friendlyName: "Input",
			pinType: "Input",
			dataType: "Execution",
			index: 1,
		}),
		"docs-layer-child-out": makePin({
			id: "docs-layer-child-out",
			name: "exec_out",
			friendlyName: "Output",
			pinType: "Output",
			dataType: "Execution",
			index: 1,
			connectedTo: ["docs-transform-in"],
		}),
		"docs-layer-child-text-out": makePin({
			id: "docs-layer-child-text-out",
			name: "text",
			friendlyName: "Clean Text",
			pinType: "Output",
			dataType: "String",
			index: 2,
			connectedTo: ["docs-transform-text-in"],
		}),
	},
});

const transformNode = makeNode({
	id: "docs-transform-node",
	name: "ai_draft_reply",
	friendlyName: "Draft Helpful Reply",
	description: "Drafts a concise response using the normalized request.",
	category: "AI/Generative",
	icon: "/flow/icons/message.svg",
	coordinates: [505, 180, 0],
	layer: "docs-layer",
	hash: 10_005,
	pins: {
		"docs-transform-in": makePin({
			id: "docs-transform-in",
			name: "exec_in",
			friendlyName: "Input",
			pinType: "Input",
			dataType: "Execution",
			index: 1,
			dependsOn: ["docs-layer-child-out"],
		}),
		"docs-transform-text-in": makePin({
			id: "docs-transform-text-in",
			name: "request",
			friendlyName: "Request",
			pinType: "Input",
			dataType: "String",
			index: 2,
			dependsOn: ["docs-layer-child-text-out"],
		}),
		"docs-transform-out": makePin({
			id: "docs-transform-out",
			name: "exec_out",
			friendlyName: "Done",
			pinType: "Output",
			dataType: "Execution",
			index: 1,
		}),
		"docs-transform-text-out": makePin({
			id: "docs-transform-text-out",
			name: "reply",
			friendlyName: "Reply",
			pinType: "Output",
			dataType: "String",
			index: 2,
		}),
	},
});

const collapsedLayer = {
	id: "docs-layer",
	parent_id: null,
	name: "Prepare Support Reply",
	type: "Collapsed",
	nodes: {},
	variables: {},
	comments: {},
	coordinates: [1110, 55, 0],
	in_coordinates: [35, 190, 0],
	out_coordinates: [760, 190, 0],
	pins: {
		"docs-layer-in": makePin({
			id: "docs-layer-in",
			name: "exec_in",
			friendlyName: "Request",
			pinType: "Input",
			dataType: "Execution",
			index: 1,
			dependsOn: ["docs-event-exec-out"],
		}),
		"docs-layer-message-in": makePin({
			id: "docs-layer-message-in",
			name: "request",
			friendlyName: "Message",
			pinType: "Input",
			dataType: "String",
			index: 2,
			dependsOn: ["docs-event-request-out"],
		}),
		"docs-layer-out": makePin({
			id: "docs-layer-out",
			name: "exec_out",
			friendlyName: "Done",
			pinType: "Output",
			dataType: "Execution",
			index: 1,
			connectedTo: ["docs-placeholder-in"],
		}),
		"docs-layer-message-out": makePin({
			id: "docs-layer-message-out",
			name: "reply",
			friendlyName: "Reply",
			pinType: "Output",
			dataType: "String",
			index: 2,
		}),
	},
	comment: "Two implementation steps are grouped into one reusable layer.",
	error: null,
	color: "#3de39d",
	hash: 20_001,
};

const placeholderLayer = {
	id: "docs-placeholder-layer",
	parent_id: null,
	name: "Human Review (placeholder)",
	type: "Collapsed",
	nodes: {},
	variables: {},
	comments: {},
	coordinates: [1425, 55, 0],
	in_coordinates: [35, 180, 0],
	out_coordinates: [620, 180, 0],
	pins: {
		"docs-placeholder-in": makePin({
			id: "docs-placeholder-in",
			name: "exec_in",
			friendlyName: "Draft",
			pinType: "Input",
			dataType: "Execution",
			index: 1,
			dependsOn: ["docs-layer-out"],
		}),
		"docs-placeholder-out": makePin({
			id: "docs-placeholder-out",
			name: "exec_out",
			friendlyName: "Approved",
			pinType: "Output",
			dataType: "Execution",
			index: 1,
		}),
	},
	comment: "Prototype a future review step before implementing its internals.",
	error: null,
	color: "#f59e0b",
	hash: 20_002,
};

const boardNodes = {
	...(structuredClone(websiteBoard.nodes) as JsonRecord),
	[eventNode.id as string]: eventNode,
	[stringSourceNode.id as string]: stringSourceNode,
	[genericTargetNode.id as string]: genericTargetNode,
	[layerChildNode.id as string]: layerChildNode,
	[transformNode.id as string]: transformNode,
};

const boardComments = {
	...(structuredClone(websiteBoard.comments) as JsonRecord),
	"docs-layer-note": {
		id: "docs-layer-note",
		author: "Flow-Like Documentation",
		content:
			'plate_json::[{"children":[{"text":"Inside the collapsed layer","bold":true}],"type":"p","id":"docs-layer-title"},{"children":[{"text":"Normalize the request, then draft a helpful reply."}],"type":"p","id":"docs-layer-copy"}]',
		comment_type: "Text",
		timestamp: {
			secs_since_epoch: 1_785_301_200,
			nanos_since_epoch: 0,
		},
		coordinates: [210, 40, 0],
		width: 470,
		height: 85,
		layer: "docs-layer",
		color: "#3de39d33",
		z_index: 2,
		hash: 30_001,
		is_locked: true,
	},
};

const board = {
	...structuredClone(websiteBoard),
	id: "docs-board",
	name: "Customer Support Automation",
	description:
		"Classifies support requests, drafts helpful replies, and keeps a human in control.",
	nodes: boardNodes,
	layers: {
		"docs-layer": collapsedLayer,
		"docs-placeholder-layer": placeholderLayer,
	},
	comments: boardComments,
	variables: {
		"docs-confidence": {
			id: "docs-confidence",
			name: "Confidence",
			description: "Minimum confidence required before a reply is suggested.",
			data_type: "Float",
			value_type: "Normal",
			default_value: encodeJson(0.75),
			editable: true,
			exposed: false,
			secret: false,
			runtime_configured: false,
			category: null,
			schema: null,
			hash: 40_001,
		},
		"docs-customer-name": {
			id: "docs-customer-name",
			name: "Customer Name",
			description: "Name used to personalize the response.",
			data_type: "String",
			value_type: "Normal",
			default_value: encodeJson("Avery Morgan"),
			editable: true,
			exposed: true,
			secret: false,
			runtime_configured: false,
			category: null,
			schema: null,
			hash: 40_002,
		},
		"docs-escalation-enabled": {
			id: "docs-escalation-enabled",
			name: "Escalation Enabled",
			description: "Routes uncertain replies to a human reviewer.",
			data_type: "Boolean",
			value_type: "Normal",
			default_value: encodeJson(true),
			editable: true,
			exposed: true,
			secret: false,
			runtime_configured: false,
			category: null,
			schema: null,
			hash: 40_003,
		},
		"docs-tags": {
			id: "docs-tags",
			name: "Routing Tags",
			description: "Labels used by the support routing policy.",
			data_type: "String",
			value_type: "Array",
			default_value: encodeJson(["priority", "billing"]),
			editable: true,
			exposed: false,
			secret: false,
			runtime_configured: false,
			category: null,
			schema: null,
			hash: 40_004,
		},
	},
	version: [1, 0, 0],
	viewport: [0, 0, 1],
	execution_mode: "Local",
	stage: "Dev",
	log_level: "Debug",
	page_ids: [],
	created_at: {
		secs_since_epoch: 1_785_300_000,
		nanos_since_epoch: 0,
	},
	updated_at: {
		secs_since_epoch: 1_785_301_200,
		nanos_since_epoch: 0,
	},
};

const makeCatalogNode = (
	id: string,
	name: string,
	friendlyName: string,
	category: string,
	hash: number,
	pins: JsonRecord = {},
): JsonRecord =>
	makeNode({
		id,
		name,
		friendlyName,
		description: `${friendlyName} node`,
		category,
		coordinates: [0, 0, 0],
		pins,
		start: name === "events_simple",
		hash,
	});

const catalog = [
	makeCatalogNode(
		"docs-catalog-event",
		"events_simple",
		"Simple Event",
		"Events",
		50_001,
		{
			"docs-catalog-event-out": makePin({
				id: "docs-catalog-event-out",
				name: "exec_out",
				friendlyName: "Start",
				pinType: "Output",
				dataType: "Execution",
				index: 1,
			}),
		},
	),
	makeCatalogNode("docs-catalog-delay", "delay", "Delay", "Control", 50_002),
	makeCatalogNode(
		"docs-catalog-variable-get",
		"variable_get",
		"Get Variable",
		"Variables",
		50_003,
		{
			"docs-catalog-get-ref": makePin({
				id: "docs-catalog-get-ref",
				name: "var_ref",
				friendlyName: "Variable",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
			"docs-catalog-get-value": makePin({
				id: "docs-catalog-get-value",
				name: "value_ref",
				friendlyName: "Value",
				pinType: "Output",
				dataType: "Generic",
				index: 1,
			}),
		},
	),
	makeCatalogNode(
		"docs-catalog-variable-set",
		"variable_set",
		"Set Variable",
		"Variables",
		50_004,
		{
			"docs-catalog-set-ref": makePin({
				id: "docs-catalog-set-ref",
				name: "var_ref",
				friendlyName: "Variable",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
			"docs-catalog-set-value": makePin({
				id: "docs-catalog-set-value",
				name: "value_in",
				friendlyName: "Value",
				pinType: "Input",
				dataType: "Generic",
				index: 2,
			}),
		},
	),
	makeCatalogNode(
		"docs-mail-copy",
		"mail_copy",
		"Copy Mail Message",
		"Email",
		50_101,
		{
			"docs-mail-copy-input": makePin({
				id: "docs-mail-copy-input",
				name: "message",
				friendlyName: "Message",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
			"docs-mail-copy-output": makePin({
				id: "docs-mail-copy-output",
				name: "copy",
				friendlyName: "Copy",
				pinType: "Output",
				dataType: "String",
				index: 1,
			}),
		},
	),
	makeCatalogNode(
		"docs-mail-send",
		"mail_send",
		"Send Email",
		"Email",
		50_102,
		{
			"docs-mail-send-input": makePin({
				id: "docs-mail-send-input",
				name: "body",
				friendlyName: "Body",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
		},
	),
	makeCatalogNode(
		"docs-mail-watch",
		"mail_watch",
		"Watch Inbox",
		"Email",
		50_103,
	),
	makeCatalogNode(
		"docs-mail-parse",
		"mail_parse",
		"Parse Mailbox",
		"Email",
		50_104,
		{
			"docs-mail-parse-input": makePin({
				id: "docs-mail-parse-input",
				name: "raw_message",
				friendlyName: "Raw Message",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
		},
	),
	makeCatalogNode(
		"docs-mail-attachment",
		"mail_attachment",
		"Add Mail Attachment",
		"Email",
		50_105,
		{
			"docs-mail-attachment-input": makePin({
				id: "docs-mail-attachment-input",
				name: "path",
				friendlyName: "Path",
				pinType: "Input",
				dataType: "String",
				index: 1,
			}),
		},
	),
];

const profile = {
	id: "docs-profile",
	name: "Documentation",
	description: "A deterministic workspace for product documentation.",
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
	bits: [],
	custom_bits: [],
	settings: {
		connection_mode: "simplebezier",
	},
	created: "2026-07-29T05:00:00Z",
	updated: "2026-07-29T05:20:00Z",
};

const settingsProfile = {
	hub_profile: profile,
	execution_settings: {
		gpu_mode: false,
		max_context_size: 128_000,
	},
	created: "2026-07-29T05:00:00Z",
	updated: "2026-07-29T05:20:00Z",
};

const app = {
	id: "docs-app",
	status: "Active",
	visibility: "Offline",
	authors: ["Flow-Like Documentation"],
	bits: [],
	boards: ["docs-board"],
	events: [],
	templates: [],
	page_ids: [],
	widget_ids: [],
	packages: {},
	rating_sum: 0,
	rating_count: 0,
	avg_rating: 0,
	download_count: 0,
	interactions_count: 0,
	execution_mode: "Local",
	allow_forking: true,
	version: "1.0.0",
	changelog: "",
	primary_category: "CustomerSupport",
	secondary_category: "Productivity",
	price: null,
	created_at: board.created_at,
	updated_at: board.updated_at,
};

const metadata = {
	name: "Customer Support Automation",
	description:
		"Classify requests and prepare accurate, human-reviewed replies.",
	long_description:
		"A deterministic documentation app that demonstrates nodes, connections, layers, variables, versions, runs, and logs.",
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
	created_at: board.created_at,
	updated_at: board.updated_at,
};

const run = {
	app_id: "docs-app",
	board_id: "docs-board",
	event_id: "docs-event",
	event_version: "1.0.0",
	run_id: "docs-run-ok",
	node_id: "docs-event-node",
	start: 1_785_301_200_000_000,
	end: 1_785_301_201_850_000,
	log_level: 1,
	logs: 2,
	nodes: [
		["docs-event-node", 1],
		["docs-transform-node", 1],
	],
	payload: encodeJson({}),
	version: "1-0-0",
};

const logs = [
	{
		node_id: "docs-event-node",
		operation_id: "op-1",
		log_level: "Info",
		message: "Received onboarding request",
		start: {
			secs_since_epoch: 1_785_301_200,
			nanos_since_epoch: 0,
		},
		end: {
			secs_since_epoch: 1_785_301_200,
			nanos_since_epoch: 120_000_000,
		},
		stats: null,
	},
	{
		node_id: "docs-transform-node",
		operation_id: "op-2",
		log_level: "Info",
		message: "Drafted a helpful reply and queued human review",
		start: {
			secs_since_epoch: 1_785_301_200,
			nanos_since_epoch: 120_000_000,
		},
		end: {
			secs_since_epoch: 1_785_301_201,
			nanos_since_epoch: 850_000_000,
		},
		stats: {
			token_in: 184,
			token_out: 96,
		},
	},
];

const fixture = {
	schema: onboardingFixture.schema,
	strict: false,
	responses: {
		...onboardingFixture.responses,
		get_current_profile: settingsProfile,
		get_current_profile_id: profile.id,
		get_profiles: {
			[profile.id]: settingsProfile,
		},
		get_profiles_raw: {
			[profile.id]: settingsProfile,
		},
		get_apps: [],
		get_app: app,
		get_app_meta: metadata,
		get_app_boards: [board],
		get_board: board,
		get_catalog: catalog,
		get_events: [],
		get_app_routes: [],
		get_pages: [],
		graph_list_overlays: [],
		graph_list_remote_ontology_imports: [],
		graph_list_imports: [],
		flowpilot_list_board_edit_jobs: [],
		get_board_versions: [
			[2, 0, 0],
			[1, 3, 0],
			[1, 2, 1],
			[1, 0, 0],
		],
		list_runs: [run],
		query_run: logs,
	},
};

const outputPath = resolve(fixturesDirectory, "docs-studio.tauri.json");
await writeFile(outputPath, `${JSON.stringify(fixture, null, "\t")}\n`, "utf8");
await formatGeneratedJson(outputPath);

console.log(`Generated ${outputPath}`);
