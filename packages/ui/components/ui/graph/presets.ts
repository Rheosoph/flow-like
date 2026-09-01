import type {
	EdgeLabelMapping,
	LabelStyle,
	NodeLabelMapping,
} from "../../../state/backend-state/graph-state";

interface PresetRule {
	patterns: string[];
	icon: string;
	color: string;
}

interface EdgePresetRule {
	patterns: string[];
	color: string;
}

export interface DomainPreset {
	name: string;
	description: string;
	nodeRules: PresetRule[];
	edgeRules: EdgePresetRule[];
	fallbackNode: { icon: string; color: string };
	fallbackEdge: { color: string };
}

const PRESETS: DomainPreset[] = [
	{
		name: "People & Social",
		description: "People, organizations, and social relationships",
		nodeRules: [
			{ patterns: ["person", "user"], icon: "user", color: "#3b82f6" },
			{ patterns: ["org", "company"], icon: "building", color: "#8b5cf6" },
			{ patterns: ["team"], icon: "users", color: "#6366f1" },
		],
		edgeRules: [
			{ patterns: ["knows", "friend"], color: "#64748b" },
			{ patterns: ["member", "works"], color: "#a855f7" },
		],
		fallbackNode: { icon: "user", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Fleet / Vehicles",
		description: "Vehicles, drivers, and routes",
		nodeRules: [
			{ patterns: ["vehicle", "car"], icon: "car", color: "#ef4444" },
			{ patterns: ["truck"], icon: "truck", color: "#f97316" },
			{ patterns: ["driver"], icon: "user", color: "#3b82f6" },
			{ patterns: ["route"], icon: "mapPin", color: "#10b981" },
		],
		edgeRules: [
			{ patterns: ["drives"], color: "#3b82f6" },
			{ patterns: ["on_route", "route"], color: "#10b981" },
		],
		fallbackNode: { icon: "car", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Devices & Infra",
		description: "Devices, servers, and network infrastructure",
		nodeRules: [
			{ patterns: ["device"], icon: "smartphone", color: "#0ea5e9" },
			{ patterns: ["server"], icon: "server", color: "#6366f1" },
			{ patterns: ["router"], icon: "router", color: "#14b8a6" },
			{ patterns: ["endpoint"], icon: "cpu", color: "#8b5cf6" },
		],
		edgeRules: [
			{ patterns: ["connects"], color: "#64748b" },
			{ patterns: ["owns"], color: "#3b82f6" },
		],
		fallbackNode: { icon: "cpu", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Places & Logistics",
		description: "Locations, warehouses, and shipments",
		nodeRules: [
			{ patterns: ["location", "place"], icon: "mapPin", color: "#10b981" },
			{ patterns: ["warehouse"], icon: "building", color: "#8b5cf6" },
			{ patterns: ["shipment", "package"], icon: "package", color: "#f59e0b" },
		],
		edgeRules: [
			{ patterns: ["ships_to", "delivers"], color: "#f59e0b" },
			{ patterns: ["located_at"], color: "#10b981" },
		],
		fallbackNode: { icon: "mapPin", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Finance / Transactions",
		description: "Accounts, customers, and transactions",
		nodeRules: [
			{ patterns: ["account"], icon: "wallet", color: "#22c55e" },
			{ patterns: ["customer", "client"], icon: "user", color: "#3b82f6" },
			{ patterns: ["transaction"], icon: "creditCard", color: "#f59e0b" },
		],
		edgeRules: [
			{ patterns: ["paid", "payment"], color: "#22c55e" },
			{ patterns: ["from", "to"], color: "#64748b" },
		],
		fallbackNode: { icon: "wallet", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Docs & Knowledge",
		description: "Documents, topics, and authorship",
		nodeRules: [
			{
				patterns: ["document", "doc", "page"],
				icon: "fileText",
				color: "#6366f1",
			},
			{ patterns: ["topic", "tag", "concept"], icon: "tag", color: "#8b5cf6" },
			{ patterns: ["author", "user"], icon: "user", color: "#3b82f6" },
		],
		edgeRules: [
			{ patterns: ["mentions", "references"], color: "#64748b" },
			{ patterns: ["authored", "wrote"], color: "#8b5cf6" },
		],
		fallbackNode: { icon: "fileText", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
	{
		name: "Incidents & Events",
		description: "Incidents, events, and actors",
		nodeRules: [
			{
				patterns: ["incident", "alert"],
				icon: "alertTriangle",
				color: "#ef4444",
			},
			{ patterns: ["event"], icon: "activity", color: "#f97316" },
			{ patterns: ["actor", "user", "person"], icon: "user", color: "#3b82f6" },
		],
		edgeRules: [
			{ patterns: ["triggered", "caused"], color: "#ef4444" },
			{ patterns: ["involved"], color: "#64748b" },
		],
		fallbackNode: { icon: "alertTriangle", color: "#94a3b8" },
		fallbackEdge: { color: "#64748b" },
	},
];

export function getPresets(): DomainPreset[] {
	return PRESETS;
}

function matchesAny(label: string, patterns: string[]): boolean {
	const lower = label.toLowerCase();
	return patterns.some((p) => lower.includes(p.toLowerCase()));
}

function defaultStyle(icon: string, color: string): LabelStyle {
	return {
		color,
		icon,
		size: { mode: "by-degree", min: 8, max: 20 },
	};
}

export function applyPreset(
	preset: DomainPreset,
	nodes: NodeLabelMapping[],
	edges: EdgeLabelMapping[],
): { nodes: NodeLabelMapping[]; edges: EdgeLabelMapping[] } {
	const updatedNodes = nodes.map((n) => {
		const rule = preset.nodeRules.find((r) => matchesAny(n.label, r.patterns));
		const style = rule
			? defaultStyle(rule.icon, rule.color)
			: defaultStyle(preset.fallbackNode.icon, preset.fallbackNode.color);
		return { ...n, style };
	});

	const updatedEdges = edges.map((e) => {
		const rule = preset.edgeRules.find((r) => matchesAny(e.label, r.patterns));
		const color = rule ? rule.color : preset.fallbackEdge.color;
		return { ...e, style: defaultStyle("link", color) };
	});

	return { nodes: updatedNodes, edges: updatedEdges };
}
