import {
	ClipboardList,
	Clock,
	Code,
	Cog,
	Globe,
	Inbox,
	LayoutDashboard,
	LayoutGrid,
	Link2,
	type LucideIcon,
	Mail,
	MapPin,
	MessageCircle,
	MessageSquare,
	Plug,
	Send,
	Server,
	Table2,
	Users,
	Webhook,
	Workflow,
	Zap,
} from "lucide-react";
import { IAppType } from "../../../lib/schema/app/app";
import type {
	IAppShowcase,
	IShowcaseColumn,
	IShowcaseEvent,
	IShowcaseOutlineNode,
	IShowcaseTable,
} from "../../../lib/schema/app/showcase";

export type ShowcaseSectionKey =
	| "data"
	| "map"
	| "triggers"
	| "pages"
	| "widgets"
	| "templates"
	| "hood"
	| "activity";

export type ShowcaseNodeKind =
	| "event"
	| "flow"
	| "table"
	| "source"
	| "page"
	| "widget";

export type ShowcaseLaneKey = "trig" | "flow" | "data" | "surf";

export interface ShowcaseMapNode {
	key: string;
	kind: ShowcaseNodeKind;
	id: string;
	label: string;
	mono: boolean;
	eventType?: string;
	rows?: number | null;
	nodeCount?: number;
	route?: string | null;
	driver?: string;
}

export interface ShowcaseMapLane {
	key: ShowcaseLaneKey;
	nodes: ShowcaseMapNode[];
}

export interface ShowcaseEdge {
	from: string;
	to: string;
}

export interface ShowcaseGraph {
	lanes: ShowcaseMapLane[];
	edges: ShowcaseEdge[];
	nodes: Map<string, ShowcaseMapNode>;
}

export const nodeKey = (kind: ShowcaseNodeKind, id: string) => `${kind}:${id}`;

const ORDER: Record<string, ShowcaseSectionKey[]> = {
	[IAppType.DataFocus]: [
		"data",
		"map",
		"triggers",
		"pages",
		"widgets",
		"templates",
		"hood",
		"activity",
	],
	[IAppType.Agent]: [
		"triggers",
		"map",
		"hood",
		"pages",
		"widgets",
		"data",
		"templates",
		"activity",
	],
	[IAppType.Form]: [
		"pages",
		"triggers",
		"map",
		"data",
		"widgets",
		"templates",
		"hood",
		"activity",
	],
	[IAppType.CustomInterface]: [
		"pages",
		"triggers",
		"map",
		"data",
		"widgets",
		"templates",
		"hood",
		"activity",
	],
};

const DEFAULT_ORDER: ShowcaseSectionKey[] = [
	"map",
	"triggers",
	"data",
	"pages",
	"widgets",
	"templates",
	"hood",
	"activity",
];

export function sectionOrder(
	appType: IAppType | null | undefined,
): ShowcaseSectionKey[] {
	return (appType && ORDER[appType]) || DEFAULT_ORDER;
}

export function buildGraph(showcase: IAppShowcase): ShowcaseGraph {
	const lanes: ShowcaseMapLane[] = [
		{
			key: "trig" as const,
			nodes: showcase.events.map((e) => ({
				key: nodeKey("event", e.id),
				kind: "event" as const,
				id: e.id,
				label: e.name,
				mono: false,
				eventType: e.event_type,
			})),
		},
		{
			key: "flow" as const,
			nodes: showcase.flows.map((f) => ({
				key: nodeKey("flow", f.id),
				kind: "flow" as const,
				id: f.id,
				label: f.name,
				mono: false,
				nodeCount: f.node_count,
			})),
		},
		{
			key: "data" as const,
			nodes: [
				...showcase.tables.map((t) => ({
					key: nodeKey("table", t.name),
					kind: "table" as const,
					id: t.name,
					label: t.name,
					mono: true,
					rows: t.rows,
				})),
				...showcase.sources.map((s) => ({
					key: nodeKey("source", s.id),
					kind: "source" as const,
					id: s.id,
					label: s.name,
					mono: true,
					driver: s.driver,
				})),
			],
		},
		{
			key: "surf" as const,
			nodes: [
				...showcase.pages.map((p) => ({
					key: nodeKey("page", p.id),
					kind: "page" as const,
					id: p.id,
					label: p.name,
					mono: false,
					route: p.route,
				})),
				...showcase.widgets.map((w) => ({
					key: nodeKey("widget", w.id),
					kind: "widget" as const,
					id: w.id,
					label: w.name,
					mono: false,
				})),
			],
		},
	].filter((lane) => lane.nodes.length > 0);

	const nodes = new Map<string, ShowcaseMapNode>();
	for (const lane of lanes) for (const n of lane.nodes) nodes.set(n.key, n);

	const edges: ShowcaseEdge[] = [];
	const add = (from: string, to: string) => {
		if (
			from !== to &&
			nodes.has(from) &&
			nodes.has(to) &&
			!edges.some((e) => e.from === from && e.to === to)
		)
			edges.push({ from, to });
	};

	const hasFlows = showcase.flows.length > 0;
	for (const e of showcase.events) {
		const from = nodeKey("event", e.id);
		if (hasFlows && e.board_id) {
			add(from, nodeKey("flow", e.board_id));
			continue;
		}
		for (const t of e.tables ?? []) add(from, nodeKey("table", t));
		for (const p of e.pages ?? []) add(from, nodeKey("page", p));
		if (e.page_id) add(from, nodeKey("page", e.page_id));
	}
	for (const f of showcase.flows) {
		const from = nodeKey("flow", f.id);
		for (const t of f.tables) add(from, nodeKey("table", t));
		for (const p of f.pages) add(from, nodeKey("page", p));
	}
	return { lanes, edges, nodes };
}

export function traceSet(edges: ShowcaseEdge[], key: string): Set<string> {
	const down = new Set([key]);
	const up = new Set([key]);
	for (let changed = true; changed; ) {
		changed = false;
		for (const e of edges) {
			if (down.has(e.from) && !down.has(e.to)) {
				down.add(e.to);
				changed = true;
			}
			if (up.has(e.to) && !up.has(e.from)) {
				up.add(e.from);
				changed = true;
			}
		}
	}
	return new Set([...down, ...up]);
}

export function visibleSections(
	showcase: IAppShowcase,
	appType: IAppType | null | undefined,
): ShowcaseSectionKey[] {
	const graph = buildGraph(showcase);
	const present: Record<ShowcaseSectionKey, boolean> = {
		data: showcase.tables.length > 0 || showcase.sources.length > 0,
		map: showcase.map && graph.lanes.length >= 2,
		triggers: showcase.events.length > 0,
		pages: showcase.pages.length > 0,
		widgets: showcase.widgets.length > 0,
		templates: showcase.templates.length > 0,
		hood: !!showcase.hood,
		activity:
			!!showcase.activity &&
			((showcase.activity.runs?.length ?? 0) > 0 ||
				(showcase.activity.versions?.length ?? 0) > 0),
	};
	return sectionOrder(appType).filter((k) => present[k]);
}

export interface EventKind {
	icon: LucideIcon;
	label: [key: string, fallback: string];
	short: [key: string, fallback: string];
}

const EVENT_KINDS: Record<string, EventKind> = {
	simple_chat: {
		icon: MessageSquare,
		label: ["showcaseEventChat", "Chat"],
		short: ["showcaseEventChat", "Chat"],
	},
	api: {
		icon: Code,
		label: ["showcaseEventApi", "API endpoint"],
		short: ["showcaseEventApiShort", "API"],
	},
	http: {
		icon: Globe,
		label: ["showcaseEventHttp", "HTTP endpoint"],
		short: ["showcaseEventApiShort", "API"],
	},
	rest: {
		icon: Code,
		label: ["showcaseEventRest", "REST endpoint"],
		short: ["showcaseEventApiShort", "API"],
	},
	webhook: {
		icon: Webhook,
		label: ["showcaseEventWebhook", "Webhook"],
		short: ["showcaseEventWebhook", "Webhook"],
	},
	mcp: {
		icon: Plug,
		label: ["showcaseEventMcp", "MCP server"],
		short: ["showcaseEventMcpShort", "MCP"],
	},
	cron: {
		icon: Clock,
		label: ["showcaseEventCron", "Schedule"],
		short: ["showcaseEventCron", "Schedule"],
	},
	email: {
		icon: Mail,
		label: ["showcaseEventEmail", "Email"],
		short: ["showcaseEventEmail", "Email"],
	},
	inbound_email: {
		icon: Inbox,
		label: ["showcaseEventInboundEmail", "Email inbox"],
		short: ["showcaseEventEmail", "Email"],
	},
	discord: {
		icon: MessageCircle,
		label: ["showcaseEventDiscord", "Discord"],
		short: ["showcaseEventDiscord", "Discord"],
	},
	telegram: {
		icon: Send,
		label: ["showcaseEventTelegram", "Telegram"],
		short: ["showcaseEventTelegram", "Telegram"],
	},
	teams: {
		icon: Users,
		label: ["showcaseEventTeams", "Microsoft Teams"],
		short: ["showcaseEventTeams", "Teams"],
	},
	generic_form: {
		icon: ClipboardList,
		label: ["showcaseEventForm", "Form"],
		short: ["showcaseEventForm", "Form"],
	},
	geolocation: {
		icon: MapPin,
		label: ["showcaseEventLocation", "Location"],
		short: ["showcaseEventLocation", "Location"],
	},
	deeplink: {
		icon: Link2,
		label: ["showcaseEventDeeplink", "Link"],
		short: ["showcaseEventDeeplink", "Link"],
	},
	quick_action: {
		icon: Zap,
		label: ["showcaseEventQuickAction", "Quick action"],
		short: ["showcaseEventQuickAction", "Quick action"],
	},
	daemon: {
		icon: Cog,
		label: ["showcaseEventDaemon", "Background service"],
		short: ["showcaseEventDaemon", "Background"],
	},
};

const FALLBACK_KIND: EventKind = {
	icon: Zap,
	label: ["showcaseEventOther", "Trigger"],
	short: ["showcaseEventOther", "Trigger"],
};

export const eventKind = (type: string): EventKind =>
	EVENT_KINDS[type] ?? FALLBACK_KIND;

export const ENDPOINT_EVENT_TYPES = new Set(["api", "http", "webhook", "rest"]);

export const isEndpointEvent = (e: IShowcaseEvent) =>
	ENDPOINT_EVENT_TYPES.has(e.event_type) && !!e.technical?.path;

export type SnippetLang = "curl" | "js" | "py";

export function endpointMethod(e: IShowcaseEvent): string {
	const m = e.technical?.method?.toUpperCase();
	if (m) return m;
	return e.event_type === "webhook" ? "POST" : "GET";
}

export function endpointSnippet(e: IShowcaseEvent, lang: SnippetLang): string {
	const method = endpointMethod(e);
	const path = e.technical?.path ?? "/";
	const withBody = method !== "GET" && method !== "DELETE";
	if (lang === "curl") {
		const flag = method === "GET" ? "" : ` -X ${method}`;
		const body = withBody
			? ` \\\n  -H "Content-Type: application/json" \\\n  -d '{}'`
			: "";
		return `curl${flag} "$APP_ENDPOINT${path}" \\\n  -H "Authorization: Bearer $FLOW_LIKE_TOKEN"${body}`;
	}
	if (lang === "js") {
		const init = [
			method === "GET" ? "" : `  method: "${method}",`,
			`  headers: {\n    Authorization: \`Bearer \${FLOW_LIKE_TOKEN}\`,${withBody ? '\n    "Content-Type": "application/json",' : ""}\n  },`,
			withBody ? "  body: JSON.stringify({})," : "",
		]
			.filter(Boolean)
			.join("\n");
		return `const res = await fetch(\`\${APP_ENDPOINT}${path}\`, {\n${init}\n});\nconst data = await res.json();`;
	}
	const lines = [
		"import os, requests",
		"",
		`res = requests.${method.toLowerCase()}(`,
		`    f"{os.environ['APP_ENDPOINT']}${path}",`,
		`    headers={"Authorization": f"Bearer {os.environ['FLOW_LIKE_TOKEN']}"},`,
	];
	if (withBody) lines.push("    json={},");
	lines.push(")", "data = res.json()");
	return lines.join("\n");
}

export type ColumnFamilyKey =
	| "text"
	| "number"
	| "time"
	| "vector"
	| "geo"
	| "other";

export const FAMILY_ORDER: ColumnFamilyKey[] = [
	"text",
	"number",
	"time",
	"vector",
	"geo",
	"other",
];

export const familyOf = (c: IShowcaseColumn): ColumnFamilyKey =>
	(FAMILY_ORDER as string[]).includes(c.family)
		? (c.family as ColumnFamilyKey)
		: "other";

export const familyVar = (k: ColumnFamilyKey) => `var(--showcase-${k})`;

export function familyCounts(
	columns: IShowcaseColumn[],
): { key: ColumnFamilyKey; n: number }[] {
	const counts = new Map<ColumnFamilyKey, number>();
	for (const c of columns) {
		const k = familyOf(c);
		counts.set(k, (counts.get(k) ?? 0) + 1);
	}
	return FAMILY_ORDER.filter((k) => counts.has(k)).map((k) => ({
		key: k,
		n: counts.get(k) ?? 0,
	}));
}

export const tableTitle = (t: IShowcaseTable) => t.object_type?.name ?? t.name;

export const sumRows = (tables: IShowcaseTable[]): number | null =>
	tables.length > 0 && tables.every((t) => typeof t.rows === "number")
		? tables.reduce((a, t) => a + (t.rows ?? 0), 0)
		: null;

export const formatNumber = (n: number) => n.toLocaleString();

export function compact(n: number): string {
	if (n >= 1e6) return `${+(n / 1e6).toFixed(2)}M`;
	if (n >= 1e4) return `${Math.round(n / 1e3)}k`;
	if (n >= 1e3) return `${+(n / 1e3).toFixed(1)}k`;
	return String(n);
}

export function niceMax(m: number): number {
	if (m <= 0) return 1;
	const p = 10 ** Math.floor(Math.log10(m));
	return [1, 2, 3, 4, 5, 6, 8, 10].map((x) => x * p).find((x) => x >= m) ?? m;
}

export const PERMISSION_LABELS: Record<
	string,
	[key: string, fallback: string]
> = {
	NetworkHttp: ["showcasePermNetworkHttp", "Web requests"],
	NetworkWebsocket: ["showcasePermNetworkWebsocket", "Live connections"],
	NetworkTcp: ["showcasePermNetworkTcp", "Network (TCP)"],
	NetworkUdp: ["showcasePermNetworkUdp", "Network (UDP)"],
	NetworkDns: ["showcasePermNetworkDns", "Domain lookups"],
	StorageRead: ["showcasePermStorageRead", "Read files"],
	StorageWrite: ["showcasePermStorageWrite", "Write files"],
	Variables: ["showcasePermVariables", "App variables"],
	Cache: ["showcasePermCache", "Cache"],
	Streaming: ["showcasePermStreaming", "Streaming"],
	Models: ["showcasePermModels", "AI models"],
	A2ui: ["showcasePermA2ui", "Page interface"],
	OAuth: ["showcasePermOAuth", "Connected accounts"],
	Functions: ["showcasePermFunctions", "Run other flows"],
};

export function sqlExample(
	showcase: IAppShowcase,
	alias: string,
): { table: string; sql: string } | null {
	for (const t of showcase.tables) {
		const col = t.columns?.find((c) => familyOf(c) === "text" && !c.personal);
		if (!col) continue;
		const q = (s: string) => (/^[a-z_][a-z0-9_]*$/.test(s) ? s : `"${s}"`);
		return {
			table: t.name,
			sql: `SELECT ${q(col.name)}, COUNT(*) AS total\nFROM ${q(alias)}.${q(t.name)}\nGROUP BY ${q(col.name)}\nORDER BY total DESC\nLIMIT 10;`,
		};
	}
	return null;
}

const CONTAINER_TYPES = new Set([
	"column",
	"stack",
	"box",
	"center",
	"scrollArea",
	"aspectRatio",
	"card",
	"tabs",
	"accordion",
	"modal",
	"drawer",
	"popover",
	"tooltip",
	"overlay",
	"absolute",
	"form",
]);

export type WireKind =
	| "col"
	| "row"
	| "grid"
	| "text"
	| "heading"
	| "media"
	| "table"
	| "chart"
	| "map"
	| "button"
	| "field"
	| "list"
	| "block";

export interface WireNode {
	kind: WireKind;
	card?: boolean;
	children: WireNode[];
}

const WIRE_LEAVES: Record<string, WireKind> = {
	text: "text",
	markdown: "text",
	richText: "text",
	badge: "button",
	heading: "heading",
	image: "media",
	video: "media",
	avatar: "media",
	lottie: "media",
	iframe: "media",
	table: "table",
	tableRow: "table",
	calendar: "table",
	gantt: "table",
	chart: "chart",
	plotlyChart: "chart",
	nivoChart: "chart",
	forecastChart: "chart",
	progress: "chart",
	map: "map",
	geoMap: "map",
	button: "button",
	link: "button",
	appLink: "button",
	input: "field",
	textField: "field",
	select: "field",
	textarea: "field",
	checkbox: "field",
	switch: "field",
	slider: "field",
	dateTimeInput: "field",
	radioGroup: "field",
	fileInput: "field",
	list: "list",
	graph: "chart",
	ontologyGraph: "chart",
};

const MAX_WIRE_DEPTH = 5;
const MAX_WIRE_CHILDREN = 8;
const MAX_WIRE_NODES = 48;

export function outlineToWire(root: IShowcaseOutlineNode): WireNode {
	let budget = MAX_WIRE_NODES;
	const walk = (n: IShowcaseOutlineNode, depth: number): WireNode | null => {
		if (budget <= 0) return null;
		budget -= 1;
		const children = n.c ?? [];
		const isContainer =
			CONTAINER_TYPES.has(n.t) ||
			n.t === "row" ||
			n.t === "grid" ||
			(children.length > 0 && !WIRE_LEAVES[n.t]);
		if (!isContainer) {
			return { kind: WIRE_LEAVES[n.t] ?? "block", children: [] };
		}
		const kind: WireKind =
			n.t === "row" ? "row" : n.t === "grid" ? "grid" : "col";
		const kids =
			depth >= MAX_WIRE_DEPTH
				? []
				: children
						.slice(0, MAX_WIRE_CHILDREN)
						.map((c) => walk(c, depth + 1))
						.filter((c): c is WireNode => c !== null);
		return { kind, card: n.t === "card", children: kids };
	};
	return walk(root, 0) ?? { kind: "col", children: [] };
}

export const showcaseAccess = (price: number | null | undefined) =>
	(price ?? 0) > 0 ? "purchase" : "members";

export function nodeIcon(
	kind: ShowcaseNodeKind,
	eventType?: string,
): LucideIcon {
	switch (kind) {
		case "event":
			return eventKind(eventType ?? "").icon;
		case "flow":
			return Workflow;
		case "table":
			return Table2;
		case "source":
			return Server;
		case "page":
			return LayoutDashboard;
		default:
			return LayoutGrid;
	}
}
