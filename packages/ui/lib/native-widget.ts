import {
	DEFAULT_HOME_DATA_CONFIG,
	type HomeDataConfig,
	normalizeHomeDataConfig,
} from "../components/home/home-data-query";
import type { IBackendState } from "../state/backend-state";
import { getApiOrigin } from "./api-url";
import { type AppQueryParam, parseAppRouteTarget } from "./app-route-url";

export const NATIVE_WIDGETS_CHANGED = "flow-like:native-widgets-changed";
export const NATIVE_WIDGETS_REFRESH = "flow-like:native-widgets-refresh";
export const NATIVE_WIDGETS_UPDATED = "flow-like:native-widgets-updated";
export const MAX_NATIVE_WIDGETS = 12;
export const NATIVE_CHART_TYPES = [
	["stat", "Single metric"],
	["progress", "Target progress"],
	["gauge", "Target gauge"],
	["bar", "Columns"],
	["horizontal", "Ranked bars"],
	["stacked", "Stacked columns"],
	["line", "Line"],
	["area", "Area"],
	["donut", "Donut"],
	["pie", "Pie"],
] as const;
export type NativeChartType = (typeof NATIVE_CHART_TYPES)[number][0];
export type NativeWidgetAccent = "orange" | "blue" | "teal" | "purple";
export interface NativeWidgetOpenAction {
	kind: "open_app";
	appId: string;
	path?: string;
	queryParams?: AppQueryParam[];
}
interface NativeWidgetDefinitionBase {
	id: string;
	title: string;
	appId: string;
	path: string;
	queryParams: AppQueryParam[];
	accent: NativeWidgetAccent;
	refreshMinutes: number;
	updatedAt: string;
}
export interface NativeWidgetChartDefinition
	extends NativeWidgetDefinitionBase {
	kind: "chart";
	data: HomeDataConfig;
}
export interface NativeWidgetPageDefinition extends NativeWidgetDefinitionBase {
	kind: "page";
	containerId?: string;
}
export type NativeWidgetDefinition =
	| NativeWidgetChartDefinition
	| NativeWidgetPageDefinition;
export interface NativeWidgetImage {
	png?: string;
	text?: string;
	template?: boolean;
}
export interface NativeWidgetChart {
	type: NativeChartType;
	points: {
		id: string;
		label: string;
		series: string;
		value: number;
		formattedValue: string;
	}[];
	format: {
		style: "number" | "currency" | "percent";
		currency: string;
		decimals: number;
	};
	target?: number;
	value?: number;
	formattedValue?: string;
}
export interface NativeWidgetPageNode {
	id: string;
	kind:
		| "column"
		| "row"
		| "stack"
		| "grid"
		| "card"
		| "text"
		| "image"
		| "icon"
		| "badge"
		| "progress"
		| "divider"
		| "spacer"
		| "table"
		| "chart"
		| "link";
	text?: string;
	role?: "title" | "headline" | "body" | "caption";
	tone?: "default" | "muted" | "accent" | "success" | "warning" | "danger";
	alignment?: "leading" | "center" | "trailing";
	spacing?: number;
	columns?: number;
	children?: NativeWidgetPageNode[];
	image?: NativeWidgetImage;
	progress?: number;
	value?: string;
	table?: { columns: string[]; rows: string[][] };
	chart?: NativeWidgetChart;
	action?: NativeWidgetOpenAction;
}
export interface NativeCustomWidget {
	id: string;
	title: string;
	kind: "chart" | "page";
	appId: string;
	updatedAt: string;
	staleAt: string;
	expiresAt: string;
	state: "ready" | "empty" | "error" | "unsupported" | "unavailable";
	message?: string;
	warnings?: string[];
	accent?: NativeWidgetAccent;
	action: NativeWidgetOpenAction;
	chart?: NativeWidgetChart;
	page?: NativeWidgetPageNode;
}

export function nativeWidgetScope(
	backend: Pick<IBackendState, "profile">,
	viewerId?: string,
): string {
	return JSON.stringify([
		getApiOrigin(backend.profile),
		backend.profile?.id ?? "local",
		viewerId ?? "local",
	]);
}

export function newNativeWidgetDefinition(
	kind: "chart" | "page",
): NativeWidgetDefinition {
	const base = {
		id: crypto.randomUUID(),
		title: kind === "chart" ? "Data Chart" : "App Page",
		appId: "",
		path: "/",
		queryParams: [],
		accent: "orange" as const,
		refreshMinutes: 30,
		updatedAt: new Date().toISOString(),
	};
	return kind === "chart"
		? { ...base, kind, data: structuredClone(DEFAULT_HOME_DATA_CONFIG) }
		: { ...base, kind };
}

const record = (value: unknown): Record<string, unknown> =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
/** Keep string limits compatible with Swift's Unicode decoder. */
export function truncateNativeWidgetText(
	value: string,
	length: number,
): string {
	return value.slice(0, length).replace(/[\uD800-\uDBFF]$/, "");
}
const boundedString = (value: unknown, length: number) =>
	typeof value === "string"
		? truncateNativeWidgetText(value.trim(), length)
		: "";

export function normalizeNativeWidgetDefinition(
	value: unknown,
): NativeWidgetDefinition {
	const raw = record(value);
	const id = boundedString(raw.id, 80);
	if (!/^[a-zA-Z0-9_-]{1,80}$/.test(id))
		throw new Error("This widget has an invalid identifier.");
	if (raw.kind !== "chart" && raw.kind !== "page")
		throw new Error("Choose a supported widget type.");
	const title = boundedString(raw.title, 120);
	if (!title) throw new Error("Give this widget a name.");
	const data = normalizeHomeDataConfig(record(raw.data));
	const appId = boundedString(
		raw.kind === "chart" ? data.appId : raw.appId,
		512,
	);
	if (!appId) throw new Error("Choose an app for this widget.");
	const target = parseAppRouteTarget(
		typeof raw.path === "string" ? raw.path : "/",
		raw.queryParams,
	);
	const refreshMinutes =
		typeof raw.refreshMinutes === "number" &&
		Number.isFinite(raw.refreshMinutes)
			? Math.min(360, Math.max(15, Math.floor(raw.refreshMinutes)))
			: 30;
	const accent = ["orange", "blue", "teal", "purple"].includes(
		String(raw.accent),
	)
		? (raw.accent as NativeWidgetAccent)
		: "orange";
	const updated =
		typeof raw.updatedAt === "string" ? new Date(raw.updatedAt) : new Date();
	if (!Number.isFinite(updated.getTime()))
		throw new Error("This widget has an invalid revision.");
	const base = {
		id,
		title,
		appId,
		path: target.path ?? "/",
		queryParams: target.queryParams,
		refreshMinutes,
		accent,
		updatedAt: updated.toISOString(),
	};
	if (raw.kind === "page")
		return {
			...base,
			kind: "page",
			containerId: boundedString(raw.containerId, 256) || undefined,
		};
	if (!NATIVE_CHART_TYPES.some(([type]) => type === data.visualization))
		throw new Error("Choose a chart supported by native widgets.");
	if (
		(data.sourceKind === "table" && !data.table) ||
		(data.sourceKind === "ontology" &&
			(!data.ontologyId || !data.objectType)) ||
		(data.sourceKind === "query" && !data.queryId)
	)
		throw new Error("Choose the data source for this chart.");
	if (
		["progress", "gauge"].includes(data.visualization) &&
		(data.target === null || data.target <= 0)
	)
		throw new Error("Set a positive target for this widget.");
	const scalar = ["stat", "progress", "gauge"].includes(data.visualization);
	return {
		...base,
		kind: "chart",
		data: {
			...data,
			appId,
			mode: "aggregate",
			limit: Math.min(data.limit, 120),
			refreshSeconds: 0,
			...(scalar
				? { groupBy: "", seriesBy: "", measures: data.measures.slice(0, 1) }
				: {}),
		},
	};
}

export interface NativeWidgetStorage {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}
const key = (scope: string) =>
	`flow-like:native-widget-definitions:v1:${scope}`;
export function readNativeWidgetDefinitions(
	scope: string,
	storage?: NativeWidgetStorage,
): NativeWidgetDefinition[] {
	try {
		const raw = (storage ?? globalThis.localStorage).getItem(key(scope));
		if (!raw || raw.length > 262_144) return [];
		const values: unknown = JSON.parse(raw);
		if (!Array.isArray(values)) return [];
		const seen = new Set<string>();
		return values.slice(0, MAX_NATIVE_WIDGETS).flatMap((value) => {
			try {
				const definition = normalizeNativeWidgetDefinition(value);
				if (seen.has(definition.id)) return [];
				seen.add(definition.id);
				return [definition];
			} catch {
				return [];
			}
		});
	} catch {
		return [];
	}
}

export function saveNativeWidgetDefinitions(
	scope: string,
	definitions: NativeWidgetDefinition[],
	storage?: NativeWidgetStorage,
) {
	if (definitions.length > MAX_NATIVE_WIDGETS)
		throw new Error(
			`You can save up to ${MAX_NATIVE_WIDGETS} native widgets on this device.`,
		);
	const values = definitions.map(normalizeNativeWidgetDefinition);
	if (new Set(values.map((value) => value.id)).size !== values.length)
		throw new Error("Widget identifiers must be unique.");
	const encoded = JSON.stringify(values);
	if (new TextEncoder().encode(encoded).length > 262_144)
		throw new Error("These widget settings are too large to save.");
	(storage ?? globalThis.localStorage).setItem(key(scope), encoded);
	if (typeof window !== "undefined")
		window.dispatchEvent(
			new CustomEvent(NATIVE_WIDGETS_CHANGED, { detail: { scope } }),
		);
}

export function nativeWidgetShell(
	definition: NativeWidgetDefinition,
	now = new Date(),
): NativeCustomWidget {
	return {
		id: definition.id,
		title: definition.title,
		kind: definition.kind,
		appId: definition.appId,
		updatedAt: now.toISOString(),
		staleAt: new Date(
			now.getTime() + definition.refreshMinutes * 60_000,
		).toISOString(),
		expiresAt: new Date(now.getTime() + 7 * 86_400_000).toISOString(),
		state: "unavailable",
		accent: definition.accent,
		action: {
			kind: "open_app",
			appId: definition.appId,
			path: definition.path,
			queryParams: definition.queryParams,
		},
	};
}
