import type { IMetadata } from "../../../../lib/schema/bit/bit";
import {
	type IBoard,
	type ILayer,
	type INode,
	IPinType,
	IVariableType,
} from "../../../../lib/schema/flow/board";

/** `[appId, templateId, metadata]` as `getTemplates` returns it. */
export type ITemplateEntry = readonly [string, string, IMetadata | undefined];

export type ITemplateSort = "updated" | "name" | "nodes";

/** What the gallery reads from a template board. */
export interface ITemplateDigest {
	/** A template is a copy of its flow, so the board name is the flow's name at snapshot time. */
	readonly sourceName: string;
	readonly version: readonly [number, number, number];
	readonly nodeCount: number;
	readonly layerCount: number;
	readonly variableCount: number;
	readonly entryNames: readonly string[];
	/** Distinct node names, most used first. */
	readonly nodeTypes: readonly string[];
}

export interface ITemplateItem {
	readonly key: string;
	readonly appId: string;
	readonly templateId: string;
	readonly meta: IMetadata | undefined;
	readonly name: string;
	readonly description: string;
	readonly tags: readonly string[];
	/** Empty until the board loaded. */
	readonly sourceName: string;
	readonly updatedAtMs: number;
	readonly board: IBoard | undefined;
	readonly boardState: "loading" | "ready" | "error";
	readonly digest: ITemplateDigest | undefined;
}

export interface ISourceGroup {
	readonly name: string;
	readonly count: number;
}

export type IThumbnailKind =
	| "event"
	| "ai"
	| "data"
	| "logic"
	| "io"
	| "other"
	| "layer";

export interface IThumbnailBox {
	readonly id: string;
	readonly x: number;
	readonly y: number;
	readonly w: number;
	readonly h: number;
	readonly kind: IThumbnailKind;
}

export interface IThumbnailWire {
	readonly id: string;
	readonly d: string;
	readonly execution: boolean;
}

export interface IThumbnail {
	readonly viewBox: string;
	readonly boxes: readonly IThumbnailBox[];
	readonly comments: readonly IThumbnailBox[];
	readonly wires: readonly IThumbnailWire[];
}

const NODE_WIDTH = 220;
const HEADER_HEIGHT = 36;
const PIN_ROW = 18;
const MAX_PIN_ROWS = 8;
const VIEW_PADDING = 48;
const MIN_VIEW_WIDTH = 900;
const MIN_VIEW_HEIGHT = 420;
const VIEW_ASPECT = 2.1;
const MAX_BOXES = 500;
const MAX_WIRES = 1000;

const KIND_BY_CATEGORY: readonly [string, IThumbnailKind][] = [
	["event", "event"],
	["ai", "ai"],
	["llm", "ai"],
	["ml", "ai"],
	["data", "data"],
	["database", "data"],
	["db", "data"],
	["sql", "data"],
	["control", "logic"],
	["logic", "logic"],
	["math", "logic"],
	["utils", "logic"],
	["variable", "logic"],
	["web", "io"],
	["http", "io"],
	["io", "io"],
	["storage", "io"],
	["file", "io"],
	["bit", "io"],
];

function kindOf(node: INode): IThumbnailKind {
	if (node.start) return "event";
	const head = (node.category ?? "").split("/")[0]?.trim().toLowerCase() ?? "";
	for (const [prefix, kind] of KIND_BY_CATEGORY) {
		if (head === prefix || head.startsWith(`${prefix} `)) return kind;
	}
	return "other";
}

function boxHeight(pins: INode["pins"] | ILayer["pins"]): number {
	let inputs = 0;
	let outputs = 0;
	for (const pin of Object.values(pins ?? {})) {
		if (pin.pin_type === IPinType.Input) inputs += 1;
		else outputs += 1;
	}
	return (
		HEADER_HEIGHT + Math.min(MAX_PIN_ROWS, Math.max(inputs, outputs)) * PIN_ROW
	);
}

function point(coordinates: readonly number[] | null | undefined) {
	return {
		x: Number.isFinite(coordinates?.[0]) ? (coordinates?.[0] as number) : 0,
		y: Number.isFinite(coordinates?.[1]) ? (coordinates?.[1] as number) : 0,
	};
}

function isTopLevel(layer: string | null | undefined): boolean {
	return !layer;
}

/** The top-level layer that contains `layerId`, or undefined for a broken parent chain. */
function topLayerOf(
	layerId: string,
	layers: Record<string, ILayer>,
): string | undefined {
	const seen = new Set<string>();
	let current: string | undefined = layerId;
	while (current && !seen.has(current)) {
		seen.add(current);
		const layer: ILayer | undefined = layers[current];
		if (!layer) return undefined;
		if (!layer.parent_id) return current;
		current = layer.parent_id;
	}
	return undefined;
}

export function digestTemplate(board: IBoard): ITemplateDigest {
	const nodes = Object.values(board.nodes ?? {});
	const counts = new Map<string, number>();
	const entryNames: string[] = [];
	for (const node of nodes) {
		const label = node.friendly_name || node.name;
		if (!label) continue;
		counts.set(label, (counts.get(label) ?? 0) + 1);
		if (node.start) entryNames.push(label);
	}
	const nodeTypes = [...counts.entries()]
		.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
		.map(([label]) => label);
	const version = board.version ?? [0, 0, 0];
	return {
		sourceName: board.name ?? "",
		version: [version[0] ?? 0, version[1] ?? 0, version[2] ?? 0],
		nodeCount: nodes.length,
		layerCount: Object.keys(board.layers ?? {}).length,
		variableCount: Object.keys(board.variables ?? {}).length,
		entryNames: [...new Set(entryNames)],
		nodeTypes,
	};
}

/** First published versions are 0.0.0, which says nothing worth a chip. */
export function versionLabel(
	version: readonly [number, number, number] | undefined,
): string | undefined {
	if (!version || version.every((part) => part === 0)) return undefined;
	return `v${version.join(".")}`;
}

/**
 * The top level of a board as boxes and wires, for a card-sized SVG. Nodes inside
 * layers are folded into their top-level layer, the way the editor shows them.
 */
export function buildThumbnail(board: IBoard): IThumbnail | undefined {
	const layers = board.layers ?? {};
	const nodes = Object.values(board.nodes ?? {});
	const boxes: IThumbnailBox[] = [];
	const boxById = new Map<string, IThumbnailBox>();

	const push = (box: IThumbnailBox) => {
		if (boxes.length >= MAX_BOXES) return;
		boxes.push(box);
		boxById.set(box.id, box);
	};

	for (const node of nodes) {
		if (!isTopLevel(node.layer)) continue;
		const { x, y } = point(node.coordinates);
		push({
			id: node.id,
			x,
			y,
			w: NODE_WIDTH,
			h: boxHeight(node.pins),
			kind: kindOf(node),
		});
	}
	for (const layer of Object.values(layers)) {
		if (layer.parent_id) continue;
		const { x, y } = point(layer.coordinates);
		push({
			id: layer.id,
			x,
			y,
			w: NODE_WIDTH,
			h: boxHeight(layer.pins),
			kind: "layer",
		});
	}
	if (boxes.length === 0) return undefined;

	const ownerOfPin = new Map<string, string>();
	for (const node of nodes) {
		const owner = isTopLevel(node.layer)
			? node.id
			: topLayerOf(node.layer as string, layers);
		if (!owner || !boxById.has(owner)) continue;
		for (const pinId of Object.keys(node.pins ?? {}))
			ownerOfPin.set(pinId, owner);
	}

	const wires: IThumbnailWire[] = [];
	const seenWires = new Set<string>();
	for (const node of nodes) {
		const from = isTopLevel(node.layer)
			? node.id
			: topLayerOf(node.layer as string, layers);
		const source = from ? boxById.get(from) : undefined;
		if (!source) continue;
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.pin_type !== IPinType.Output) continue;
			for (const target of pin.connected_to ?? []) {
				if (wires.length >= MAX_WIRES) break;
				const targetBox = boxById.get(ownerOfPin.get(target) ?? "");
				if (!targetBox || targetBox.id === source.id) continue;
				const key = `${source.id}>${targetBox.id}`;
				if (seenWires.has(key)) continue;
				seenWires.add(key);
				const sx = source.x + source.w;
				const sy = source.y + Math.min(source.h / 2, HEADER_HEIGHT);
				const tx = targetBox.x;
				const ty = targetBox.y + Math.min(targetBox.h / 2, HEADER_HEIGHT);
				const bend = Math.max(40, Math.abs(tx - sx) / 2);
				wires.push({
					id: key,
					d: `M${sx} ${sy} C${sx + bend} ${sy} ${tx - bend} ${ty} ${tx} ${ty}`,
					execution: pin.data_type === IVariableType.Execution,
				});
			}
		}
	}

	let minX = Number.POSITIVE_INFINITY;
	let minY = Number.POSITIVE_INFINITY;
	let maxX = Number.NEGATIVE_INFINITY;
	let maxY = Number.NEGATIVE_INFINITY;
	for (const box of boxes) {
		minX = Math.min(minX, box.x);
		minY = Math.min(minY, box.y);
		maxX = Math.max(maxX, box.x + box.w);
		maxY = Math.max(maxY, box.y + box.h);
	}

	const comments: IThumbnailBox[] = [];
	for (const comment of Object.values(board.comments ?? {})) {
		if (!isTopLevel(comment.layer)) continue;
		const { x, y } = point(comment.coordinates);
		const w = comment.width ?? 0;
		const h = comment.height ?? 0;
		if (w <= 0 || h <= 0) continue;
		if (x > maxX || y > maxY || x + w < minX || y + h < minY) continue;
		comments.push({ id: comment.id, x, y, w, h, kind: "other" });
	}

	let width = maxX - minX + VIEW_PADDING * 2;
	let height = maxY - minY + VIEW_PADDING * 2;
	const centerX = (minX + maxX) / 2;
	const centerY = (minY + maxY) / 2;
	width = Math.max(width, MIN_VIEW_WIDTH, height * VIEW_ASPECT);
	height = Math.max(height, MIN_VIEW_HEIGHT, width / VIEW_ASPECT);

	return {
		viewBox: [centerX - width / 2, centerY - height / 2, width, height]
			.map((value) => Math.round(value))
			.join(" "),
		boxes,
		comments,
		wires,
	};
}

export function toTemplateItem(
	entry: ITemplateEntry,
	board: IBoard | undefined,
	boardState: ITemplateItem["boardState"],
): ITemplateItem {
	const [appId, templateId, meta] = entry;
	const digest = board ? digestTemplate(board) : undefined;
	const updated = meta?.updated_at ?? meta?.created_at;
	return {
		key: `${appId}/${templateId}`,
		appId,
		templateId,
		meta,
		name: meta?.name || templateId,
		description: meta?.description ?? "",
		tags: meta?.tags ?? [],
		sourceName: digest?.sourceName ?? "",
		updatedAtMs: updated
			? updated.secs_since_epoch * 1000 +
				Math.floor((updated.nanos_since_epoch ?? 0) / 1e6)
			: 0,
		board,
		boardState,
		digest,
	};
}

export function sourceGroups(items: readonly ITemplateItem[]): ISourceGroup[] {
	const counts = new Map<string, number>();
	for (const item of items) {
		if (!item.sourceName) continue;
		counts.set(item.sourceName, (counts.get(item.sourceName) ?? 0) + 1);
	}
	return [...counts.entries()]
		.map(([name, count]) => ({ name, count }))
		.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

export function filterBySource(
	items: readonly ITemplateItem[],
	source: string | null,
): ITemplateItem[] {
	if (!source) return [...items];
	return items.filter((item) => item.sourceName === source);
}

export function sortTemplates(
	items: readonly ITemplateItem[],
	sort: ITemplateSort,
): ITemplateItem[] {
	const sorted = [...items];
	if (sort === "name") {
		sorted.sort(
			(a, b) =>
				a.name.localeCompare(b.name) ||
				a.sourceName.localeCompare(b.sourceName),
		);
	} else if (sort === "nodes") {
		sorted.sort(
			(a, b) =>
				(b.digest?.nodeCount ?? -1) - (a.digest?.nodeCount ?? -1) ||
				b.updatedAtMs - a.updatedAtMs,
		);
	} else {
		sorted.sort(
			(a, b) => b.updatedAtMs - a.updatedAtMs || a.name.localeCompare(b.name),
		);
	}
	return sorted;
}

/** "Card Generator (Backup 04.11)": the default name for a flow made from a template. */
export function defaultFlowName(item: ITemplateItem): string {
	return item.sourceName && item.sourceName !== item.name
		? `${item.sourceName} (${item.name})`
		: item.name;
}
