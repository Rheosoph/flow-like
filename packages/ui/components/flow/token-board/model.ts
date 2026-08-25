"use client";

/**
 * The data model behind the sidebar token board.
 *
 * Variables and function layers are rendered as typed chips rather than rows, so
 * the panel needs three things the old list never computed: a type/container
 * vocabulary, a predicate filter, and a reference count per item. All three are
 * derived — nothing here is persisted on the board.
 */

import { owningModuleId } from "../../../lib/layer-to-function";
import type { IBoard, ILayer, IVariable } from "../../../lib/schema/flow/board";
import { ILayerType } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import {
	type ICategoryNode,
	buildCategoryTree,
	compareByNameThenId,
	normalizeCategory,
} from "../category-tree";

/** One literal glyph per data type — read the type without learning nine hues. */
export const TOKEN_GLYPH: Record<string, string> = {
	[IVariableType.String]: '"',
	[IVariableType.Integer]: "#",
	[IVariableType.Float]: "≈",
	[IVariableType.Boolean]: "01",
	[IVariableType.Byte]: "0x",
	[IVariableType.Struct]: "{}",
	[IVariableType.Date]: "◷",
	[IVariableType.Generic]: "?",
	[IVariableType.PathBuf]: "/",
	[IVariableType.Execution]: "▶",
};

const TOKEN_COLOR_SLUG: Record<string, string> = {
	[IVariableType.String]: "string",
	[IVariableType.Integer]: "integer",
	[IVariableType.Float]: "float",
	[IVariableType.Boolean]: "boolean",
	[IVariableType.Byte]: "byte",
	[IVariableType.Struct]: "struct",
	[IVariableType.Date]: "date",
	[IVariableType.Generic]: "generic",
	[IVariableType.PathBuf]: "pathbuf",
	[IVariableType.Execution]: "execution",
};

/** The chip fill for a data type, as a CSS custom property reference. */
export const tokenColor = (type: IVariableType | "Function"): string =>
	type === "Function"
		? "var(--tok-function)"
		: `var(--tok-${TOKEN_COLOR_SLUG[type] ?? "generic"})`;

/** The ink that reads on {@link tokenColor}. */
export const tokenInk = (type: IVariableType | "Function"): string =>
	type === "Function"
		? "var(--tok-function-ink)"
		: `var(--tok-${TOKEN_COLOR_SLUG[type] ?? "generic"}-ink)`;

export const containerFormClass = (value_type: IValueType): string => {
	if (value_type === IValueType.Array) return "fl-tok--array";
	if (value_type === IValueType.HashSet) return "fl-tok--set";
	return "";
};

export const isFunctionLayer = (layer: ILayer): boolean =>
	layer.type === ILayerType.Function;

export const functionLayers = (board: IBoard): ILayer[] =>
	Object.values(board.layers ?? {}).filter(isFunctionLayer);

/* ── usage ─────────────────────────────────────────────────────────────── */

export interface IUsageIndex {
	/** variable id → number of Get/Set nodes that reference it */
	variables: Record<string, number>;
	/** function layer id → number of Call nodes that reference it */
	functions: Record<string, number>;
}

const EMPTY_USAGE: IUsageIndex = { variables: {}, functions: {} };

/** Reads a node's reference pin (`var_ref`, `function_layer_id`) as a plain id. */
export const refFromPin = (
	node: { pins?: Record<string, unknown> },
	pinName: string,
): string | undefined => {
	for (const pin of Object.values(node.pins ?? {})) {
		const p = pin as { name?: string; default_value?: number[] | null };
		if (p?.name !== pinName) continue;
		const value = parseUint8ArrayToJson(p.default_value);
		return typeof value === "string" && value.length > 0 ? value : undefined;
	}
	return undefined;
};

/**
 * Counts references by walking the board's nodes once.
 *
 * A variable is referenced by `variable_get` / `variable_set` nodes through their
 * `var_ref` pin, and a function by `control_call_function` through
 * `function_layer_id` — the same encoding the placement code writes, so the count
 * is exact rather than a heuristic.
 */
export function buildUsageIndex(board: IBoard | undefined): IUsageIndex {
	if (!board?.nodes) return EMPTY_USAGE;

	const variables: Record<string, number> = {};
	const functions: Record<string, number> = {};

	for (const node of Object.values(board.nodes)) {
		const name = (node as { name?: string }).name;
		if (name === "variable_get" || name === "variable_set") {
			const ref = refFromPin(node as never, "var_ref");
			if (ref) variables[ref] = (variables[ref] ?? 0) + 1;
			continue;
		}
		if (name === "control_call_function") {
			const ref = refFromPin(node as never, "function_layer_id");
			if (ref) functions[ref] = (functions[ref] ?? 0) + 1;
		}
	}

	return { variables, functions };
}

/**
 * Which scope actually holds a variable.
 *
 * Both scopes render in one board while you are inside a function layer, so a
 * drop's droppable id cannot say which one an item came from — only a lookup
 * can, and getting this wrong silently discards the move.
 */
export function resolveVariableScope(
	id: string,
	local: Record<string, IVariable> | undefined,
	board: Record<string, IVariable> | undefined,
): "local" | "board" | null {
	if (local?.[id]) return "local";
	if (board?.[id]) return "board";
	return null;
}

/* ── filter ────────────────────────────────────────────────────────────── */

export interface ITokenQuery {
	text: string[];
	type: string | null;
	is: string[];
	in: string | null;
}

export const EMPTY_QUERY: ITokenQuery = {
	text: [],
	type: null,
	is: [],
	in: null,
};

/** Supported `is:` predicates, in the order the help popover lists them. */
export const IS_PREDICATES = [
	"unused",
	"used",
	"exposed",
	"secret",
	"runtime",
	"locked",
	"single",
	"array",
	"set",
	"map",
	"cached",
	"local",
] as const;

export function parseTokenQuery(raw: string): ITokenQuery {
	const query: ITokenQuery = { text: [], type: null, is: [], in: null };
	for (const token of raw.trim().split(/\s+/)) {
		if (!token) continue;
		const match = /^(type|is|in):(.+)$/i.exec(token);
		if (!match) {
			query.text.push(token.toLowerCase());
			continue;
		}
		const key = match[1].toLowerCase();
		const value = match[2].toLowerCase();
		if (key === "type") query.type = value;
		else if (key === "is") query.is.push(value);
		else query.in = value;
	}
	return query;
}

export const isQueryEmpty = (query: ITokenQuery): boolean =>
	query.text.length === 0 &&
	query.type === null &&
	query.is.length === 0 &&
	query.in === null;

/** Subsequence match, so `msc` still finds `mYSEARCHCLAUSE`. */
export function fuzzyMatch(name: string, term: string): boolean {
	const haystack = name.toLowerCase();
	if (haystack.includes(term)) return true;
	let i = 0;
	for (const char of haystack) {
		if (char === term[i]) i += 1;
		if (i === term.length) return true;
	}
	return false;
}

const inFolder = (category: string | null | undefined, needle: string) =>
	(normalizeCategory(category) ?? "top level").toLowerCase().includes(needle);

export function matchesVariable(
	variable: IVariable,
	query: ITokenQuery,
	uses: number,
	scope: "board" | "local",
): boolean {
	if (query.type && variable.data_type.toLowerCase() !== query.type)
		return false;
	if (query.in && !inFolder(variable.category, query.in)) return false;

	for (const predicate of query.is) {
		switch (predicate) {
			case "unused":
				if (uses !== 0) return false;
				break;
			case "used":
				if (uses === 0) return false;
				break;
			case "exposed":
				if (!variable.exposed) return false;
				break;
			case "secret":
				if (!variable.secret) return false;
				break;
			case "runtime":
				if (!variable.runtime_configured) return false;
				break;
			case "locked":
				if (variable.editable) return false;
				break;
			case "single":
				if (variable.value_type !== IValueType.Normal) return false;
				break;
			case "array":
				if (variable.value_type !== IValueType.Array) return false;
				break;
			case "set":
				if (variable.value_type !== IValueType.HashSet) return false;
				break;
			case "map":
				if (variable.value_type !== IValueType.HashMap) return false;
				break;
			case "local":
				if (scope !== "local") return false;
				break;
			default:
				return false;
		}
	}

	return query.text.every((term) => fuzzyMatch(variable.name, term));
}

export function matchesFunction(
	layer: ILayer,
	query: ITokenQuery,
	calls: number,
): boolean {
	if (query.type && query.type !== "function") return false;
	if (query.in && !inFolder(layer.category, query.in)) return false;

	for (const predicate of query.is) {
		switch (predicate) {
			case "unused":
				if (calls !== 0) return false;
				break;
			case "used":
				if (calls === 0) return false;
				break;
			case "cached":
				if (!layer.cache?.enabled) return false;
				break;
			default:
				return false;
		}
	}

	return query.text.every((term) => fuzzyMatch(layer.name, term));
}

/* ── grouping ──────────────────────────────────────────────────────────── */

export type IGroupMode = "folder" | "type" | "scope" | "usage";

export interface ITokenItem {
	id: string;
	name: string;
	category?: string | null;
	kind: "variable" | "function";
	variable?: IVariable;
	layer?: ILayer;
	uses: number;
	scope: "board" | "local";
}

/** A flat bucket, used by every grouping mode except `folder`. */
export interface ITokenGroup {
	key: string;
	label: string;
	items: ITokenItem[];
	/** Non-null when the group is a folder and therefore a drop target. */
	dropPath: string | null;
	tone?: "warn";
	note?: string;
}

const usageBucket = (uses: number): string =>
	uses === 0 ? "unused" : uses >= 8 ? "hot" : uses >= 3 ? "warm" : "cold";

const USAGE_ORDER = ["unused", "hot", "warm", "cold"] as const;

const TYPE_ORDER = [
	IVariableType.String,
	IVariableType.Integer,
	IVariableType.Float,
	IVariableType.Boolean,
	IVariableType.Struct,
	IVariableType.Date,
	IVariableType.Byte,
	IVariableType.Generic,
	IVariableType.PathBuf,
	IVariableType.Execution,
];

export function groupFlat(
	items: ITokenItem[],
	mode: Exclude<IGroupMode, "folder">,
	labels: {
		usage: Record<string, string>;
		local: string;
		board: string;
		function: string;
	},
): ITokenGroup[] {
	const buckets = new Map<string, ITokenGroup>();
	const push = (
		key: string,
		label: string,
		item: ITokenItem,
		tone?: "warn",
	) => {
		let group = buckets.get(key);
		if (!group) {
			group = { key, label, items: [], dropPath: null, tone };
			buckets.set(key, group);
		}
		group.items.push(item);
	};

	for (const item of items) {
		if (mode === "type") {
			const type = item.variable?.data_type ?? "Function";
			push(
				`type:${type}`,
				type === "Function" ? labels.function : String(type),
				item,
			);
			continue;
		}
		if (mode === "scope") {
			push(
				`scope:${item.scope}`,
				item.scope === "local" ? labels.local : labels.board,
				item,
			);
			continue;
		}
		const bucket = usageBucket(item.uses);
		push(
			`usage:${bucket}`,
			labels.usage[bucket] ?? bucket,
			item,
			bucket === "unused" ? "warn" : undefined,
		);
	}

	const order = (group: ITokenGroup): number => {
		if (mode === "usage")
			return USAGE_ORDER.indexOf(
				group.key.slice("usage:".length) as (typeof USAGE_ORDER)[number],
			);
		if (mode === "type") {
			const type = group.key.slice("type:".length);
			const index = TYPE_ORDER.indexOf(type as IVariableType);
			return index < 0 ? TYPE_ORDER.length : index;
		}
		return group.key === "scope:local" ? 0 : 1;
	};

	return [...buckets.values()]
		.sort((a, b) => order(a) - order(b))
		.map((group) => ({
			...group,
			items: [...group.items].sort(compareByNameThenId),
		}));
}

/* ── modules ───────────────────────────────────────────────────────────── */

/** Items that share an owning module — `null` is the board root, `main.flow`. */
export interface IModuleGroup {
	moduleId: string | null;
	items: ITokenItem[];
}

/**
 * Splits items by the module that owns them, global first and then `moduleOrder`.
 *
 * A function is module-local when its parent chain reaches a Module layer, so the split is
 * derived from the layer tree rather than stored on the item. Empty groups are dropped: a
 * module without functions is a file, not a section.
 */
export function groupItemsByModule(
	items: ITokenItem[],
	layers: Record<string, ILayer> | undefined,
	moduleOrder: readonly string[],
): IModuleGroup[] {
	const global: ITokenItem[] = [];
	const owned = new Map<string, ITokenItem[]>();

	for (const item of items) {
		const moduleId = owningModuleId(layers, item.id);
		if (!moduleId) {
			global.push(item);
			continue;
		}
		const bucket = owned.get(moduleId);
		if (bucket) bucket.push(item);
		else owned.set(moduleId, [item]);
	}

	const groups: IModuleGroup[] = [];
	if (global.length > 0) groups.push({ moduleId: null, items: global });
	for (const moduleId of moduleOrder) {
		const bucket = owned.get(moduleId);
		if (bucket?.length) groups.push({ moduleId, items: bucket });
	}
	return groups;
}

/* ── nested folders ────────────────────────────────────────────────────── */

export interface IFolderNode {
	/** The last path segment — what the header shows. */
	name: string;
	/** The full `a/b/c` path — what a drop re-files into. */
	path: string;
	depth: number;
	items: ITokenItem[];
	children: IFolderNode[];
	/** Items in this folder and everything below it. */
	total: number;
}

const toFolderNode = (
	node: ICategoryNode<ITokenItem>,
	depth: number,
): IFolderNode => {
	const children = Object.keys(node.children)
		.sort((a, b) => a.localeCompare(b))
		.map((key) => toFolderNode(node.children[key], depth + 1));
	const items = [...node.items].sort(compareByNameThenId);
	return {
		name: node.name,
		path: node.path,
		depth,
		items,
		children,
		total: items.length + children.reduce((sum, child) => sum + child.total, 0),
	};
};

/**
 * Builds the nested folder tree the board renders.
 *
 * `category` is a `"/"`-separated path, so folders genuinely nest — the board
 * indents each level by a rail rather than nesting boxes, which is what keeps a
 * three-level category from eating the panel's width.
 */
export function buildFolderTree(items: ITokenItem[]): IFolderNode {
	// The synthetic root is never rendered, so its children are the depth-0 level
	// the board sticks and indents against.
	return toFolderNode(buildCategoryTree(items), -1);
}

/** Every folder path in the tree, for pickers and move menus. */
export function folderPaths(node: IFolderNode): string[] {
	return node.children.flatMap((child) => [child.path, ...folderPaths(child)]);
}

/** The slice of dnd-kit's draggable a token hands to its shell. */
export type IDraggable = ReturnType<
	typeof import("@dnd-kit/core").useDraggable
>;
