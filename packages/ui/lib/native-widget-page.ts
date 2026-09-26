import { applyElementUpdate } from "../components/a2ui/apply-a2ui-message";
import { resolveHidden } from "../components/a2ui/resolve-hidden";
import type {
	DataEntry,
	Surface,
	SurfaceComponent,
} from "../components/a2ui/types";
import type {
	IPage,
	IPageState,
	IWidgetRef,
} from "../state/backend-state/page-state";
import type { IStorageState } from "../state/backend-state/storage-state";
import {
	APP_QUERY_PARAM,
	type AppQueryParam,
	parseAppRouteTarget,
} from "./app-route-url";
import {
	isStorageAssetPath,
	normalizeStorageAssetPath,
} from "./asset-url-cache";
import {
	type NativeWidgetChart,
	type NativeWidgetOpenAction,
	type NativeWidgetPageNode,
	truncateNativeWidgetText,
} from "./native-widget";
import { notificationIconSource } from "./notification-icon";
import { pageSurfaceRevision } from "./page-surface-cache";
import { asArray } from "./response-shape";

export const NATIVE_WIDGET_PAGE_CAPTURE =
	"flow-like:native-widget-page-capture";

/** Legacy host routing controls are not app-owned widget query parameters. */
export function nativeWidgetPageQuery(hostSearch: string): string {
	const params = new URLSearchParams(hostSearch);
	if (params.has(APP_QUERY_PARAM)) return params.get(APP_QUERY_PARAM) ?? "";
	for (const key of ["id", "route", "event"]) params.delete(key);
	return params.toString();
}

export interface NativeWidgetPageProjection {
	root?: NativeWidgetPageNode;
	warnings: string[];
	/** Ephemeral sources. The publisher replaces them with bounded image bytes. */
	media: Array<{ nodeId: string; source: string }>;
	supported: boolean;
	requiresCapture?: boolean;
}

export interface NativeWidgetPageTarget {
	appId: string;
	path?: string;
	queryParams?: AppQueryParam[];
	containerId?: string;
}

export interface NativeWidgetPageCaptureDetail {
	scope: string;
	definitionId: string;
	revision: string;
	result: NativeWidgetPageProjection;
	capturedAt: number;
	pageId: string;
	pageRevision: string;
}

/** Resolve durable app image paths under the publisher's current authenticated backend. */
export async function resolveNativeWidgetPageMedia(
	storageState: Pick<IStorageState, "downloadStorageItems">,
	appId: string,
	projection: NativeWidgetPageProjection,
	signal?: AbortSignal,
): Promise<NativeWidgetPageProjection> {
	const assertActive = () => {
		if (signal?.aborted)
			throw new DOMException("Widget refresh cancelled", "AbortError");
	};
	assertActive();
	const icons = new Set<string>();
	const collect = (node: NativeWidgetPageNode) => {
		if (node.kind === "icon") icons.add(node.id);
		for (const child of node.children ?? []) collect(child);
	};
	if (projection.root) collect(projection.root);
	const storage = projection.media
		.slice(0, 4)
		.filter(
			(item) =>
				!icons.has(item.nodeId) &&
				notificationIconSource(item.source)?.kind !== "emoji" &&
				isStorageAssetPath(item.source),
		);
	if (!storage.length) return projection;
	const paths = [
		...new Set(storage.map((item) => normalizeStorageAssetPath(item.source))),
	];
	let results: Awaited<ReturnType<IStorageState["downloadStorageItems"]>> = [];
	let cancel = () => {};
	try {
		const query = storageState.downloadStorageItems(appId, paths);
		const aborted = new Promise<never>((_, reject) => {
			cancel = () =>
				reject(new DOMException("Widget refresh cancelled", "AbortError"));
			signal?.addEventListener("abort", cancel, { once: true });
			if (signal?.aborted) cancel();
		});
		results = await Promise.race([query, aborted]);
	} catch {
		assertActive();
		// Unavailable images do not hide the page's text.
	} finally {
		signal?.removeEventListener("abort", cancel);
	}
	assertActive();
	const urls = new Map(
		asArray(results)
			.filter((item) => item?.url && !item.error)
			.map((item) => [item.prefix, item.url as string]),
	);
	const storageIds = new Set(storage.map((item) => item.nodeId));
	let failed = false;
	const media = projection.media.slice(0, 4).flatMap((item) => {
		if (!storageIds.has(item.nodeId)) return [item];
		const url = urls.get(normalizeStorageAssetPath(item.source));
		if (!url) {
			failed = true;
			return [];
		}
		return [{ ...item, source: url }];
	});
	return {
		...projection,
		media,
		warnings: failed
			? [
					...projection.warnings,
					"Some page images could not be loaded from app storage.",
				].slice(0, 12)
			: projection.warnings,
	};
}

type RecordValue = Record<string, unknown>;
type Scope = { item?: unknown; index?: number };
const unsafeKey = (key: string) =>
	["__proto__", "constructor", "prototype"].includes(key);
const record = (value: unknown): RecordValue | undefined =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as RecordValue)
		: undefined;
const primitive = (value: unknown): string | undefined => {
	if (typeof value === "string") return truncateNativeWidgetText(value, 1000);
	if (typeof value === "number" && Number.isFinite(value)) return String(value);
	if (typeof value === "boolean") return value ? "Yes" : "No";
	return undefined;
};

function parts(path: string): string[] {
	return path
		.trim()
		.replace(/^\$\.?/, "")
		.replace(/^\//, "")
		.replace(/\[(\d+)\]/g, ".$1")
		.split(/[./]/)
		.filter(Boolean);
}

function atPath(value: unknown, path: string): unknown {
	let current = value;
	for (const key of parts(path)) {
		if (
			unsafeKey(key) ||
			current === null ||
			typeof current !== "object" ||
			!Object.hasOwn(current, key)
		)
			return undefined;
		current = (current as RecordValue)[key];
	}
	return current;
}

function dataFromEntries(entries: DataEntry[] = []): RecordValue {
	const data: RecordValue = Object.create(null);
	for (const entry of entries.slice(0, 1000)) {
		const keys = parts(entry.path);
		if (!keys.length || keys.length > 32 || keys.some(unsafeKey)) continue;
		let current = data;
		for (const key of keys.slice(0, -1)) {
			const previous = record(current[key]);
			current[key] = previous ? { ...previous } : Object.create(null);
			current = current[key] as RecordValue;
		}
		current[keys[keys.length - 1]] = entry.value;
	}
	return data;
}

/** Resolves the display binding forms shared by static pages and captured surfaces. */
export function resolveNativeWidgetBinding(
	value: unknown,
	data: RecordValue = {},
	scope: Scope = {},
): unknown {
	const bound = record(value);
	if (!bound) return value;
	for (const key of [
		"literalString",
		"literalNumber",
		"literalBool",
		"literalOptions",
	]) {
		if (Object.hasOwn(bound, key)) return bound[key];
	}
	if (typeof bound.literalJson === "string") {
		try {
			return bound.literalJson.length <= 1_000_000
				? JSON.parse(bound.literalJson)
				: undefined;
		} catch {
			return undefined;
		}
	}
	if (typeof bound.path === "string") {
		let path = bound.path;
		if (!path) return bound.defaultValue;
		if (path === "$index") return scope.index ?? bound.defaultValue;
		if (scope.index !== undefined)
			path = path.replaceAll("$index", String(scope.index));
		const value = /^\$item(?:[./]|$)/.test(path)
			? atPath(scope.item, path.slice(5))
			: atPath(data, path);
		return value === undefined ? bound.defaultValue : value;
	}
	return undefined;
}

function number(value: unknown, fallback = 0): number {
	const parsed =
		typeof value === "number"
			? value
			: typeof value === "string" && value.trim()
				? Number(value)
				: Number.NaN;
	return Number.isFinite(parsed) ? parsed : fallback;
}

function spacing(value: unknown): number | undefined {
	if (typeof value === "number") return Math.max(0, Math.min(24, value));
	if (typeof value !== "string") return undefined;
	const match = value.match(/^(\d+(?:\.\d+)?)(px|rem)?$/);
	return match
		? Math.min(24, Number(match[1]) * (match[2] === "rem" ? 16 : 1))
		: undefined;
}

function plainMarkdown(text: string): string {
	return text
		.replace(/<[^>]*>/g, "")
		.replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
		.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
		.replace(/^\s{0,3}#{1,6}\s+/gm, "")
		.replace(/[*_`~]/g, "")
		.trim();
}

function exposedComponents(
	definition: IWidgetRef,
	values: RecordValue | undefined,
	updates: RecordValue | undefined,
): SurfaceComponent[] {
	const components = structuredClone(definition.components.slice(0, 500));
	for (const entry of (definition.exposedProps ?? []).slice(0, 100)) {
		const prop = record(entry);
		if (
			!prop ||
			typeof prop.id !== "string" ||
			typeof prop.targetComponentId !== "string" ||
			typeof prop.propertyPath !== "string" ||
			!values ||
			!Object.hasOwn(values, prop.id)
		)
			continue;
		const keys = prop.propertyPath.split(".").filter(Boolean);
		if (!keys.length || keys.length > 12 || keys.some(unsafeKey)) continue;
		const component = components.find(
			(item) => item.id === prop.targetComponentId,
		);
		if (!component) continue;
		let target = (keys[0] === "style"
			? component
			: component.component) as unknown as RecordValue;
		for (const key of keys.slice(0, -1)) {
			target[key] = record(target[key]) ?? {};
			target = target[key] as RecordValue;
		}
		const value = values[prop.id];
		target[keys[keys.length - 1]] =
			record(value) ||
			["TailwindClass", "StyleObject", "Json"].includes(String(prop.propType))
				? value
				: prop.propType === "Number"
					? { literalNumber: number(value) }
					: prop.propType === "Boolean"
						? { literalBool: Boolean(value) }
						: { literalString: primitive(value) ?? "" };
	}
	if (!updates) return components;
	return components.map((component) => {
		const patches = updates[component.id];
		if (!Array.isArray(patches)) return component;
		return patches
			.slice(0, 100)
			.reduce<SurfaceComponent>(
				(current, patch) =>
					record(patch) ? applyElementUpdate(current, patch) : current,
				component,
			);
	});
}

export function projectNativeWidgetPage(
	page: IPage,
	target: NativeWidgetPageTarget,
	options: { surface?: Surface; data?: RecordValue } = {},
): NativeWidgetPageProjection {
	const result: NativeWidgetPageProjection = {
		warnings: [],
		media: [],
		supported: false,
	};
	const warn = (message: string) => {
		if (result.warnings.length < 12 && !result.warnings.includes(message))
			result.warnings.push(message);
	};
	const route = parseAppRouteTarget(target.path, target.queryParams);
	const fallback: NativeWidgetOpenAction = {
		kind: "open_app",
		appId: target.appId,
		path: route.path,
		queryParams: route.queryParams,
	};
	const surface = options.surface;
	const source =
		surface?.components ??
		Object.fromEntries(
			(page.components ?? []).map((component) => [component.id, component]),
		);
	const data = options.data ?? dataFromEntries(surface?.dataModel);
	let count = 0;
	let supported = 0;
	let contentBytes = 0;
	const reserve = (node: NativeWidgetPageNode): boolean => {
		const size = new TextEncoder().encode(JSON.stringify(node)).length + 32;
		if (contentBytes + size > 48 * 1024) {
			warn(
				"This page contains too much content for a widget. Select a smaller container.",
			);
			return false;
		}
		contentBytes += size;
		return true;
	};
	const visit = (
		componentId: string,
		components: Record<string, SurfaceComponent>,
		depth: number,
		ancestors: Set<SurfaceComponent>,
		scope: Scope,
	): NativeWidgetPageNode | undefined => {
		if (count >= 60 || depth > 8) {
			warn(
				"This page is larger than an iOS widget. Select a smaller container.",
			);
			return undefined;
		}
		const wrapped = Object.hasOwn(components, componentId)
			? components[componentId]
			: Object.hasOwn(source, componentId)
				? source[componentId]
				: undefined;
		if (!wrapped || !record(wrapped.component)) {
			warn(
				"Some page elements are unavailable. Open the page to refresh its content.",
			);
			return undefined;
		}
		if (ancestors.has(wrapped)) {
			warn("A recursive page element cannot be shown in a widget.");
			return undefined;
		}
		const component = wrapped.component as unknown as RecordValue;
		const resolve = (value: unknown) =>
			resolveNativeWidgetBinding(value, data, scope);
		const read = (key: string) => resolve(component[key]);
		const text = (key: string) => primitive(read(key));
		const style = record(wrapped.style ?? component.style) ?? {};
		const className =
			typeof style.className === "string" ? style.className : "";
		if (
			resolveHidden(component.hidden, resolve) ||
			/(?:^|\s)(?:hidden|invisible)(?:\s|$)/.test(className) ||
			style.display === "none" ||
			style.visibility === "hidden" ||
			style.visibility === "collapse"
		)
			return undefined;
		if (
			className ||
			style.position ||
			style.transform ||
			style.animation ||
			style.background
		)
			warn(
				"Page styling is adapted to native widget typography, spacing and colors.",
			);
		const nextAncestors = new Set(ancestors).add(wrapped);
		const node: NativeWidgetPageNode = { id: `n${++count}`, kind: "text" };
		const align = text("align") ?? style.textAlign;
		if (align === "center") node.alignment = "center";
		else if (align === "right" || align === "end") node.alignment = "trailing";
		else if (align === "left" || align === "start") node.alignment = "leading";
		const color = String(text("color") ?? style.color ?? "");
		if (/success|green|emerald/i.test(color)) node.tone = "success";
		else if (/warning|amber|yellow/i.test(color)) node.tone = "warning";
		else if (/destructive|danger|red/i.test(color)) node.tone = "danger";
		else if (/primary|accent|orange/i.test(color)) node.tone = "accent";
		else if (/muted|secondary|gray|grey|slate/i.test(color))
			node.tone = "muted";
		const childNodes = (): NativeWidgetPageNode[] => {
			const children = record(component.children);
			if (Array.isArray(children?.explicitList))
				return children.explicitList
					.slice(0, 61)
					.flatMap((id) =>
						typeof id === "string"
							? (visit(id, components, depth + 1, nextAncestors, scope) ?? [])
							: [],
					);
			const template = record(children?.template);
			if (
				!template ||
				typeof template.dataPath !== "string" ||
				typeof template.templateComponentId !== "string"
			)
				return [];
			const values = resolve({ path: template.dataPath });
			if (!Array.isArray(values)) {
				warn(
					"Some page data is not loaded. Open the page in Flow Like to update the widget.",
				);
				return [];
			}
			if (values.length > 30)
				warn("Repeated page content is limited to 30 items in a widget.");
			return values
				.slice(0, 30)
				.flatMap(
					(item, index) =>
						visit(
							template.templateComponentId as string,
							components,
							depth + 1,
							nextAncestors,
							{ item, index },
						) ?? [],
				);
		};
		const type = component.type;
		if (
			["column", "row", "stack", "grid", "card", "box", "center"].includes(
				String(type),
			)
		) {
			node.kind =
				type === "box" || type === "center"
					? "column"
					: (type as NativeWidgetPageNode["kind"]);
			node.spacing = spacing(read("gap") ?? style.gap);
			if (type === "center") node.alignment = "center";
			if (type === "grid")
				node.columns = Math.max(
					1,
					Math.min(6, Math.trunc(number(read("columns"), 2))),
				);
			if (type === "card") {
				node.text = text("title");
				const description = text("description");
				node.value = description
					? truncateNativeWidgetText(description, 80)
					: undefined;
			}
			if (!reserve(node)) return undefined;
			if (node.text || node.value) supported++;
			node.children = childNodes();
			if (read("reverse") === true) node.children.reverse();
			return node.children.length || node.text || node.value ? node : undefined;
		}
		if (type === "widgetInstance") {
			const ref =
				typeof component.instanceId === "string" &&
				page.widgetRefs &&
				Object.hasOwn(page.widgetRefs, component.instanceId)
					? page.widgetRefs?.[component.instanceId]
					: undefined;
			if (!ref) {
				warn(
					"A reusable page widget is unavailable. Open the page in Flow Like.",
				);
				return undefined;
			}
			const expanded = exposedComponents(
				ref,
				record(component.exposedPropValues),
				record(component.runtimeChildUpdates),
			);
			const children = Object.fromEntries(
				expanded.map((item) => [item.id, item]),
			);
			return visit(
				ref.rootComponentId,
				children,
				depth + 1,
				nextAncestors,
				scope,
			);
		}
		if (["text", "markdown", "badge", "tableCell"].includes(String(type))) {
			node.kind = type === "badge" ? "badge" : "text";
			node.text = text("content");
			if (type === "markdown" && node.text)
				node.text = plainMarkdown(node.text);
			if (node.text === undefined) {
				warn(
					"Some page data is not loaded. Open the page in Flow Like to update the widget.",
				);
				return undefined;
			}
			const variant = text("variant");
			const size = text("size");
			node.role =
				variant === "heading"
					? "title"
					: ["xl", "2xl", "3xl", "4xl"].includes(size ?? "")
						? "headline"
						: variant === "caption" || size === "xs"
							? "caption"
							: "body";
		} else if (["image", "avatar", "icon"].includes(String(type))) {
			node.kind = type === "icon" ? "icon" : "image";
			const rawSource =
				type === "icon"
					? read("name")
					: (read("src") ?? (type === "image" ? read("fallback") : undefined));
			const source =
				typeof rawSource === "string" && rawSource.length <= 4_194_304
					? rawSource
					: undefined;
			node.text =
				type === "avatar"
					? text("fallback")
					: type === "image"
						? text("alt")
						: undefined;
			if (source && result.media.length < 4)
				result.media.push({
					nodeId: node.id,
					source:
						type === "icon"
							? source
									.trim()
									.replace(/^lucide:/, "")
									.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
									.replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
									.replace(/[_\s]+/g, "-")
									.toLowerCase()
							: source,
				});
			else if (source) warn("Widgets can display up to four page images.");
			else if (!node.text) {
				warn("A page image is not loaded. Open the page to update it.");
				return undefined;
			}
		} else if (type === "progress") {
			const value = number(read("value"), Number.NaN);
			if (!Number.isFinite(value)) {
				warn(
					"Progress data is not loaded. Open the page to update the widget.",
				);
				return undefined;
			}
			node.kind = "progress";
			node.progress = Math.max(
				0,
				Math.min(1, value / Math.max(0.000001, number(read("max"), 100))),
			);
			node.value = `${Math.round(node.progress * 100)}%`;
		} else if (type === "divider" || type === "spacer") {
			node.kind = type;
			node.spacing = spacing(read("size"));
		} else if (type === "table") {
			const columns = read("columns");
			const rows = read("data");
			if (!Array.isArray(columns) || !Array.isArray(rows)) {
				warn("Table data is not loaded. Open the page to update the widget.");
				return undefined;
			}
			const visible = columns
				.filter((column) => record(column) && resolve(column.hidden) !== true)
				.slice(0, 6) as RecordValue[];
			if (!visible.length) {
				warn("This table has no visible columns to show in the widget.");
				return undefined;
			}
			node.kind = "table";
			node.table = {
				columns: visible.map((column) =>
					truncateNativeWidgetText(
						primitive(resolve(column.header)) ?? primitive(column.id) ?? "",
						50,
					),
				),
				rows: rows
					.slice(0, 8)
					.map((row) =>
						visible.map((column) =>
							truncateNativeWidgetText(
								primitive(
									atPath(
										row,
										primitive(resolve(column.accessor)) ??
											primitive(column.id) ??
											"",
									),
								) ?? "",
								80,
							),
						),
					),
			};
			if (columns.length > 6 || rows.length > 8)
				warn("Widget tables show up to eight rows and six columns.");
		} else if (type === "nivoChart") {
			const chart = projectChart(read);
			if (!chart) {
				warn(
					"This chart type or its data cannot be displayed in an iOS widget.",
				);
				return undefined;
			}
			node.kind = "chart";
			node.text = text("title");
			node.chart = chart;
		} else if (["button", "link", "appLink"].includes(String(type))) {
			node.kind = "link";
			node.text = text("label") ?? "Open in Flow Like";
			if (read("disabled") !== true) node.action = fallback;
			if (
				type === "link" &&
				!component.external &&
				(text("route") || text("href"))
			) {
				try {
					const query = record(read("queryParams"));
					const route = parseAppRouteTarget(
						text("route") || text("href"),
						query
							? Object.entries(query).map(([name, value]) => ({
									name,
									value: primitive(value) ?? "",
								}))
							: undefined,
					);
					if (read("disabled") !== true)
						node.action = { kind: "open_app", appId: target.appId, ...route };
				} catch {
					warn("External links open with the full page in Flow Like.");
				}
			}
			if (type !== "link" || component.actions || component.eventHandlers)
				warn(
					"Page buttons open the app. Workflows run when you activate them inside Flow Like.",
				);
		} else {
			warn(
				`${truncateNativeWidgetText(String(type ?? "Unknown"), 48)} elements are available inside the app. Select a supported container for this widget.`,
			);
			return undefined;
		}
		if (!reserve(node)) return undefined;
		supported++;
		return node;
	};
	const root =
		target.containerId ||
		surface?.rootComponentId ||
		(source.root ? "root" : page.components?.[0]?.id);
	if (root) result.root = visit(root, source, 1, new Set(), {});
	else warn("This page has no content to show in a widget.");
	const renderedIds = new Set<string>();
	const collect = (node: NativeWidgetPageNode) => {
		renderedIds.add(node.id);
		for (const child of node.children ?? []) collect(child);
	};
	if (result.root) collect(result.root);
	result.media = result.media.filter((item) => renderedIds.has(item.nodeId));
	if (page.onLoadEventId || page.onIntervalEventId) {
		result.requiresCapture = true;
		if (!surface)
			warn(
				"Open this page in Flow Like to update content produced by its Events.",
			);
	}
	result.supported = supported > 0;
	return result;
}

function projectChart(
	read: (key: string) => unknown,
): NativeWidgetChart | undefined {
	const type = read("chartType");
	if (!["bar", "line", "area", "pie"].includes(String(type))) return undefined;
	const data = read("data");
	if (!Array.isArray(data)) return undefined;
	const format = { style: "number" as const, currency: "USD", decimals: 0 };
	const points: NativeWidgetChart["points"] = [];
	const add = (label: unknown, series: unknown, value: unknown) => {
		const numeric = number(value, Number.NaN);
		if (points.length < 120 && Number.isFinite(numeric))
			points.push({
				id: `p${points.length}`,
				label: truncateNativeWidgetText(primitive(label) ?? "", 100),
				series: truncateNativeWidgetText(primitive(series) ?? "Value", 100),
				value: numeric,
				formattedValue: new Intl.NumberFormat(undefined, {
					maximumFractionDigits: 2,
					notation: Math.abs(numeric) >= 1e15 ? "scientific" : "standard",
				}).format(numeric),
			});
	};
	if (type === "pie") {
		if (data.some((item) => record(item) && number(item.value, 0) < 0))
			return undefined;
		for (const item of data.slice(0, 120)) {
			const row = record(item);
			if (row) add(row.label ?? row.id, "Value", row.value);
		}
	} else if (type === "line" || type === "area") {
		for (const item of data.slice(0, 6)) {
			const row = record(item);
			if (!row || !Array.isArray(row.data)) continue;
			for (const point of row.data.slice(0, 120)) {
				const value = record(point);
				if (value) add(value.x, row.id, value.y);
			}
		}
	} else {
		const indexBy = primitive(read("indexBy")) ?? "id";
		const declaredKeys = read("keys");
		const keys = Array.isArray(declaredKeys)
			? declaredKeys
					.filter(
						(key): key is string => typeof key === "string" && !unsafeKey(key),
					)
					.slice(0, 6)
			: ["value"];
		for (const item of data.slice(0, 120)) {
			const row = record(item);
			if (row) for (const key of keys) add(atPath(row, indexBy), key, row[key]);
		}
	}
	return points.length
		? { type: type as NativeWidgetChart["type"], points, format }
		: undefined;
}

/** Bootstrap checks the viewer's current Page access without running lifecycle Events. */
export async function loadNativeWidgetPage(
	pageState: Pick<IPageState, "getPageBootstrap">,
	target: NativeWidgetPageTarget,
): Promise<
	NativeWidgetPageProjection & {
		pageId: string;
		pageRevision: string;
		title: string;
		canonicalRoute: string;
	}
> {
	const route = parseAppRouteTarget(target.path, target.queryParams);
	const bootstrap = await pageState.getPageBootstrap(
		target.appId,
		route.path ?? "/",
	);
	if (bootstrap.routeMiss)
		throw new Error("This app path does not match a page.");
	if (!bootstrap.page) throw new Error("This app path has no page to display.");
	const canonicalRoute = bootstrap.canonicalRoute || route.path || "/";
	return {
		...projectNativeWidgetPage(bootstrap.page, {
			...target,
			path: canonicalRoute,
			queryParams: route.queryParams,
		}),
		pageId: bootstrap.page.id,
		pageRevision:
			pageSurfaceRevision(
				bootstrap.revision ?? bootstrap.page.updatedAt,
				bootstrap.executionRevision,
			) ?? bootstrap.page.updatedAt,
		title: bootstrap.page.title || bootstrap.page.name,
		canonicalRoute,
	};
}
