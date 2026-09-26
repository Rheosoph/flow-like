import MiniSearch from "minisearch";
import { findDropTargetPin } from "../../lib/flow-board-helpers";
import {
	type IBoard,
	type ILayer,
	ILayerType,
} from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../lib/schema/flow/pin";
import type { IVariable } from "../../lib/schema/flow/variable";
import { convertJsonToUint8Array } from "../../lib/uint8";
import { categorySegments, compareByNameThenId } from "./category-tree";

export const PALETTE_MAX_RESULTS = 50;
const PALETTE_RECENT_ROWS = 6;
const PINNED_CATEGORIES = ["Control", "Variables", "Events", "Functions"];

export interface PaletteEntry {
	/** Stable across sessions: the catalog node name, or a board-scoped id for generated entries. */
	key: string;
	node: INode;
	path: string[];
}

export interface PaletteCategory {
	key: string;
	name: string;
	path: string[];
	count: number;
	icon?: string;
	children: PaletteCategory[];
	entries: PaletteEntry[];
}

export interface PaletteModel {
	entries: PaletteEntry[];
	byKey: Map<string, PaletteEntry>;
	tree: PaletteCategory;
	index: MiniSearch<PaletteDoc>;
}

export interface PaletteBoardInputs {
	startNodes: INode[];
	variables: IVariable[];
	functionLayers: ILayer[];
}

export interface PaletteLabels {
	call: (name: string) => string;
	get: (name: string) => string;
	set: (name: string) => string;
}

/** Where a wire dropped on the canvas lands on a candidate node; `target` is absent for function references. */
export interface PaletteDropFit {
	target?: IPin;
}

export type PaletteDropIndex = Map<string, PaletteDropFit>;

interface PaletteDoc {
	id: string;
	name: string;
	friendly_name: string;
	category: string;
	description: string;
	pin_in_names: string;
	pin_out_names: string;
}

// MiniSearch's default tokenizer drops punctuation, so operator names like
// "/" or "!=" would never be indexed nor matched. Keep operator runs as tokens.
const SPACE_OR_PUNCTUATION = /[\n\r\p{Z}\p{P}]+/u;
const OPERATOR_RUN = /[+\-*/%^!<>=&|~]+/g;
export function tokenizeWithOperators(text: string): string[] {
	const tokens = text.split(SPACE_OR_PUNCTUATION).filter(Boolean);
	const operators = text.match(OPERATOR_RUN);
	if (operators) tokens.push(...operators);
	return tokens;
}

export function collectBoardInputs(
	board: IBoard | undefined,
	currentLayerId: string | undefined,
): PaletteBoardInputs {
	if (!board) return { startNodes: [], variables: [], functionLayers: [] };
	const variables = Object.values(board.variables);
	if (currentLayerId) {
		const layer = board.layers[currentLayerId];
		if (layer?.type === ILayerType.Function) {
			variables.push(...Object.values(layer.variables));
		}
	}
	return {
		startNodes: Object.values(board.nodes)
			.filter((node) => node.start)
			.sort(
				(a, b) =>
					a.friendly_name.localeCompare(b.friendly_name) ||
					a.id.localeCompare(b.id),
			),
		variables: variables.sort(compareByNameThenId),
		functionLayers: Object.values(board.layers)
			.filter((layer) => layer.type === ILayerType.Function)
			.sort(compareByNameThenId),
	};
}

/** Content key for {@link collectBoardInputs}: the board object is replaced on every edit. */
export function boardInputsKey(inputs: PaletteBoardInputs): string {
	return JSON.stringify([
		inputs.startNodes.map((node) => [node.id, node.friendly_name]),
		inputs.variables.map((variable) => [
			variable.id,
			variable.name,
			variable.data_type,
			variable.value_type,
			variable.schema ?? null,
		]),
		inputs.functionLayers.map((layer) => [layer.id, layer.name]),
	]);
}

function withPinValue(node: INode, pinName: string, value: string): INode {
	const pins = Object.fromEntries(
		Object.values(node.pins).map((pin) => [
			pin.id,
			pin.name === pinName
				? { ...pin, default_value: convertJsonToUint8Array(value) }
				: pin,
		]),
	);
	return { ...node, pins };
}

/** A Get/Set Variable node bound to `variable`, with its value pins typed like the variable. */
export function bindVariableNode(
	base: INode,
	variable: IVariable,
	friendlyName: string,
): INode {
	const bound = withPinValue(base, "var_ref", variable.id);
	const pins = Object.fromEntries(
		Object.values(bound.pins).map((pin) => [
			pin.id,
			pin.name === "value_in" || pin.name === "value_ref"
				? {
						...pin,
						data_type: variable.data_type,
						value_type: variable.value_type,
						schema: variable.schema ?? null,
					}
				: pin,
		]),
	);
	return { ...bound, friendly_name: friendlyName, pins };
}

const compareEntries = (a: PaletteEntry, b: PaletteEntry) =>
	a.node.friendly_name.localeCompare(b.node.friendly_name) ||
	a.node.category.localeCompare(b.node.category);

const entryOf = (key: string, node: INode): PaletteEntry => ({
	key,
	node,
	path: categorySegments(node.category),
});

export function buildPaletteEntries(
	nodes: INode[],
	inputs: PaletteBoardInputs,
	labels: PaletteLabels,
): PaletteEntry[] {
	const byKey = new Map<string, PaletteEntry>();
	// Ontology bindings clone a prototype and keep its runtime `name`, and WASM nodes use
	// their name as id in every package, so identity is package + id. The first node of a
	// name keeps the bare key that recents store.
	const seen = new Set<string>();
	for (const node of nodes) {
		const scope = node.wasm?.package_id ? `${node.wasm.package_id}:` : "";
		const identity = `${scope}${node.id}`;
		if (seen.has(identity)) continue;
		seen.add(identity);
		const key = byKey.has(node.name) ? `${node.name}#${identity}` : node.name;
		byKey.set(key, entryOf(key, node));
	}
	const base = (name: string) => byKey.get(name)?.node;
	const add = (key: string, node: INode) =>
		byKey.set(key, entryOf(key, { ...node, id: key }));

	const callReference = base("control_call_reference");
	if (callReference) {
		for (const start of inputs.startNodes) {
			add(`call:${start.id}`, {
				...withPinValue(callReference, "fn_ref", start.id),
				friendly_name: labels.call(start.friendly_name),
				category: "Events/Call",
			});
		}
	}

	const getVariable = base("variable_get");
	const setVariable = base("variable_set");
	if (getVariable && setVariable) {
		for (const variable of inputs.variables) {
			add(`get:${variable.id}`, {
				...bindVariableNode(getVariable, variable, labels.get(variable.name)),
				category: "Variables/Get",
			});
			add(`set:${variable.id}`, {
				...bindVariableNode(setVariable, variable, labels.set(variable.name)),
				category: "Variables/Set",
			});
		}
	}

	const callFunction = base("control_call_function");
	if (callFunction) {
		for (const layer of inputs.functionLayers) {
			add(`fn:${layer.id}`, {
				...withPinValue(callFunction, "function_layer_id", layer.id),
				friendly_name: labels.call(layer.name),
				category: "Functions/Call",
			});
		}
	}

	return [...byKey.values()].sort(compareEntries);
}

const pinNames = (node: INode, type: IPinType) =>
	Object.values(node.pins)
		.filter((pin) => pin.pin_type === type)
		.map((pin) => pin.friendly_name)
		.join(" ");

function createPaletteIndex(entries: PaletteEntry[]): MiniSearch<PaletteDoc> {
	const index = new MiniSearch<PaletteDoc>({
		tokenize: tokenizeWithOperators,
		fields: [
			"name",
			"friendly_name",
			"category",
			"description",
			"pin_in_names",
			"pin_out_names",
		],
		storeFields: [],
		searchOptions: {
			prefix: true,
			fuzzy: 0.2,
			boost: {
				name: 3,
				friendly_name: 2,
				category: 1.5,
				description: 0.75,
				pin_in_names: 1,
				pin_out_names: 1,
			},
		},
	});
	index.addAll(
		entries.map(({ key, node }) => ({
			id: key,
			name: node.name,
			friendly_name: node.friendly_name,
			category: node.category,
			description: node.description ?? "",
			pin_in_names: pinNames(node, IPinType.Input),
			pin_out_names: pinNames(node, IPinType.Output),
		})),
	);
	return index;
}

interface DraftCategory extends PaletteCategory {
	kids: Map<string, DraftCategory>;
}

const draftCategory = (path: string[]): DraftCategory => ({
	key: path.join("/"),
	name: path[path.length - 1] ?? "",
	path,
	count: 0,
	children: [],
	entries: [],
	kids: new Map(),
});

const pinnedRank = (name: string) => {
	const rank = PINNED_CATEGORIES.indexOf(name);
	return rank < 0 ? PINNED_CATEGORIES.length : rank;
};

function finalizeCategory(
	category: DraftCategory,
	isRoot: boolean,
): Map<string, number> {
	const icons = new Map<string, number>();
	for (const entry of category.entries) {
		const icon = entry.node.icon;
		if (icon) icons.set(icon, (icons.get(icon) ?? 0) + 1);
	}
	for (const kid of category.kids.values()) {
		for (const [icon, count] of finalizeCategory(kid, false)) {
			icons.set(icon, (icons.get(icon) ?? 0) + count);
		}
	}
	let best = 0;
	for (const [icon, count] of icons) {
		if (count > best) {
			best = count;
			category.icon = icon;
		}
	}
	category.entries.sort(compareEntries);
	category.children = [...category.kids.values()].sort(
		(a, b) =>
			(isRoot ? pinnedRank(a.name) - pinnedRank(b.name) : 0) ||
			a.name.localeCompare(b.name),
	);
	return icons;
}

export function buildPaletteTree(entries: PaletteEntry[]): PaletteCategory {
	const root = draftCategory([]);
	for (const entry of entries) {
		let cursor = root;
		cursor.count++;
		for (const segment of entry.path) {
			let next = cursor.kids.get(segment);
			if (!next) {
				next = draftCategory([...cursor.path, segment]);
				cursor.kids.set(segment, next);
			}
			cursor = next;
			cursor.count++;
		}
		cursor.entries.push(entry);
	}
	finalizeCategory(root, true);
	return root;
}

export function findCategory(
	tree: PaletteCategory,
	path: readonly string[],
): PaletteCategory | undefined {
	let cursor: PaletteCategory | undefined = tree;
	for (const segment of path) {
		cursor = cursor?.children.find((child) => child.name === segment);
		if (!cursor) return undefined;
	}
	return cursor;
}

export function buildPaletteModel(entries: PaletteEntry[]): PaletteModel {
	return {
		entries,
		byKey: new Map(entries.map((entry) => [entry.key, entry])),
		tree: buildPaletteTree(entries),
		index: createPaletteIndex(entries),
	};
}

export const isFunctionReferencePin = (pin: IPin) =>
	pin.id.startsWith("ref_in_") || pin.id.startsWith("ref_out_");

export function dropFit(
	node: INode,
	droppedPin: IPin,
	refs: Record<string, string>,
): PaletteDropFit | undefined {
	if (droppedPin.id.startsWith("ref_in_")) {
		return node.fn_refs?.can_reference_fns ? {} : undefined;
	}
	if (droppedPin.id.startsWith("ref_out_")) {
		return node.fn_refs?.can_be_referenced_by_fns ? {} : undefined;
	}
	const target = findDropTargetPin(node.pins, droppedPin, refs);
	return target ? { target } : undefined;
}

export function buildDropIndex(
	entries: PaletteEntry[],
	droppedPin: IPin,
	refs: Record<string, string>,
): PaletteDropIndex {
	const index: PaletteDropIndex = new Map();
	for (const entry of entries) {
		const fit = dropFit(entry.node, droppedPin, refs);
		if (fit) index.set(entry.key, fit);
	}
	return index;
}

export const inScope = (entry: PaletteEntry, scope: readonly string[]) =>
	scope.every((segment, i) => entry.path[i] === segment);

const WORD_BREAK = /[^\p{L}\p{N}]+/u;

function matchTier(name: string, query: string, tokens: string[]): number {
	const lower = name.toLowerCase();
	if (lower === query) return 0;
	if (lower.startsWith(query)) return 1;
	const words = lower.split(WORD_BREAK).filter(Boolean);
	if (
		tokens.length > 0 &&
		tokens.every((token) => words.some((word) => word.startsWith(token)))
	)
		return 2;
	return 3;
}

const recentBoost = (recents: readonly string[], key: string) => {
	const rank = recents.indexOf(key);
	return rank < 0 ? 1 : 1.5 - Math.min(rank, 10) * 0.04;
};

export interface PaletteSearchOptions {
	scope: readonly string[];
	recents: readonly string[];
	accepts?: (entry: PaletteEntry) => boolean;
}

export function searchPalette(
	model: PaletteModel,
	query: string,
	{ scope, recents, accepts }: PaletteSearchOptions,
): PaletteEntry[] {
	const normalized = query.trim().toLowerCase();
	if (!normalized) return [];
	const tokens = tokenizeWithOperators(normalized);
	const ranked: { entry: PaletteEntry; tier: number; score: number }[] = [];
	for (const hit of model.index.search(normalized)) {
		const entry = model.byKey.get(String(hit.id));
		if (!entry || !inScope(entry, scope) || (accepts && !accepts(entry)))
			continue;
		ranked.push({
			entry,
			tier: matchTier(entry.node.friendly_name, normalized, tokens),
			score: hit.score * recentBoost(recents, entry.key),
		});
	}
	ranked.sort(
		(a, b) =>
			a.tier - b.tier || b.score - a.score || compareEntries(a.entry, b.entry),
	);
	return ranked.map(({ entry }) => entry);
}

/** True when every word of `query` starts a word of `label` or `keywords`. */
export function matchesWords(
	query: string,
	label: string,
	keywords = "",
): boolean {
	const tokens = query.toLowerCase().split(WORD_BREAK).filter(Boolean);
	if (tokens.length === 0) return false;
	const words = `${label} ${keywords}`
		.toLowerCase()
		.split(WORD_BREAK)
		.filter(Boolean);
	return tokens.every((token) => words.some((word) => word.startsWith(token)));
}

export function resolveRecents(
	model: PaletteModel,
	recents: readonly string[],
	accepts?: (entry: PaletteEntry) => boolean,
): PaletteEntry[] {
	const out: PaletteEntry[] = [];
	for (const key of recents) {
		const entry = model.byKey.get(key);
		if (entry && (!accepts || accepts(entry))) out.push(entry);
		if (out.length === PALETTE_RECENT_ROWS) break;
	}
	return out;
}

/** Keeps only the categories and entries `accepts` lets through, with counts recomputed. */
export function filterPaletteTree(
	model: PaletteModel,
	accepts: (entry: PaletteEntry) => boolean,
): PaletteCategory {
	return buildPaletteTree(model.entries.filter(accepts));
}

export const sortedPins = (node: INode, type: IPinType) =>
	Object.values(node.pins)
		.filter((pin) => pin.pin_type === type)
		.sort((a, b) => a.index - b.index);

export function pinTypeLabel(pin: Pick<IPin, "data_type" | "value_type">) {
	switch (pin.value_type) {
		case IValueType.Array:
			return `${pin.data_type}[]`;
		case IValueType.HashMap:
			return `Map<${pin.data_type}>`;
		case IValueType.HashSet:
			return `Set<${pin.data_type}>`;
		default:
			return pin.data_type;
	}
}

const EXECUTION_MARK = "▸";
const SIGNATURE_TYPES = 3;

function sideSignature(pins: IPin[]): string {
	const parts: string[] = [];
	if (pins.some((pin) => pin.data_type === IVariableType.Execution))
		parts.push(EXECUTION_MARK);
	for (const pin of pins) {
		if (pin.data_type !== IVariableType.Execution)
			parts.push(pinTypeLabel(pin));
	}
	return parts.length > SIGNATURE_TYPES
		? `${parts.slice(0, SIGNATURE_TYPES).join(", ")}, …`
		: parts.join(", ");
}

/** `String, Integer → Boolean`, with `▸` for execution pins. */
export function pinSignature(node: INode): string {
	const inputs = sideSignature(sortedPins(node, IPinType.Input));
	const outputs = sideSignature(sortedPins(node, IPinType.Output));
	return [inputs, "→", outputs].filter(Boolean).join(" ");
}

export type PaletteGroup =
	| "recent"
	| "recentMatches"
	| "browse"
	| "actions"
	| "nodes";

export type PaletteRow =
	| { kind: "group"; id: string; group: PaletteGroup; count?: number }
	| {
			kind: "node";
			id: string;
			entry: PaletteEntry;
			variant: "compact" | "result" | "browse";
	  }
	| { kind: "category"; id: string; category: PaletteCategory; root: boolean }
	| { kind: "action"; id: string; action: string }
	| { kind: "divider"; id: string }
	| { kind: "overflow"; id: string; shown: number; total: number }
	| { kind: "empty"; id: string; reason: "query" | "scope" }
	| { kind: "widen"; id: string; widen: "scope" | "compatible" };

export type PaletteSelectableRow = Extract<
	PaletteRow,
	{ kind: "node" | "category" | "action" | "widen" }
>;

export const isSelectableRow = (row: PaletteRow): row is PaletteSelectableRow =>
	row.kind === "node" ||
	row.kind === "category" ||
	row.kind === "action" ||
	row.kind === "widen";

export interface PaletteRowInput {
	query: string;
	scope: readonly string[];
	tree: PaletteCategory;
	recents: PaletteEntry[];
	results: PaletteEntry[];
	actions: string[];
	dropActive: boolean;
	/** A compatible-only search found nothing, but the unfiltered one would. */
	canWiden: boolean;
}

const isEmptyCategory = (category: PaletteCategory) =>
	category.children.length === 0 && category.entries.length === 0;

function categoryContents(
	category: PaletteCategory,
	root: boolean,
): PaletteRow[] {
	const rows: PaletteRow[] = category.children.map((child) => ({
		kind: "category",
		id: `category:${child.key}`,
		category: child,
		root,
	}));
	if (category.children.length > 0 && category.entries.length > 0) {
		rows.push({ kind: "divider", id: "divider" });
	}
	for (const entry of category.entries) {
		rows.push({
			kind: "node",
			id: `node:${entry.key}`,
			entry,
			variant: "browse",
		});
	}
	return rows;
}

function emptyScopeRows(scoped: boolean, dropActive: boolean): PaletteRow[] {
	const rows: PaletteRow[] = [{ kind: "empty", id: "empty", reason: "scope" }];
	if (scoped) rows.push({ kind: "widen", id: "widen:scope", widen: "scope" });
	if (dropActive) {
		rows.push({ kind: "widen", id: "widen:compatible", widen: "compatible" });
	}
	return rows;
}

export function buildPaletteRows({
	query,
	scope,
	tree,
	recents,
	results,
	actions,
	dropActive,
	canWiden,
}: PaletteRowInput): PaletteRow[] {
	const rows: PaletteRow[] = [];

	if (!query.trim()) {
		const category = findCategory(tree, scope);
		if (!category || isEmptyCategory(category)) {
			return emptyScopeRows(scope.length > 0, dropActive);
		}
		if (scope.length > 0) return categoryContents(category, false);
		if (recents.length > 0) {
			const group = dropActive ? "recentMatches" : "recent";
			rows.push({ kind: "group", id: `group:${group}`, group });
			for (const entry of recents) {
				rows.push({
					kind: "node",
					id: `recent:${entry.key}`,
					entry,
					variant: "compact",
				});
			}
		}
		rows.push({ kind: "group", id: "group:browse", group: "browse" });
		rows.push(...categoryContents(category, true));
		return rows;
	}

	if (actions.length > 0 && scope.length === 0) {
		rows.push({ kind: "group", id: "group:actions", group: "actions" });
		for (const action of actions) {
			rows.push({ kind: "action", id: `action:${action}`, action });
		}
	}

	if (results.length > 0) {
		rows.push({
			kind: "group",
			id: "group:nodes",
			group: "nodes",
			count: results.length,
		});
		for (const entry of results.slice(0, PALETTE_MAX_RESULTS)) {
			rows.push({
				kind: "node",
				id: `node:${entry.key}`,
				entry,
				variant: "result",
			});
		}
		if (results.length > PALETTE_MAX_RESULTS) {
			rows.push({
				kind: "overflow",
				id: "overflow",
				shown: PALETTE_MAX_RESULTS,
				total: results.length,
			});
		}
		return rows;
	}

	if (actions.length > 0 && scope.length === 0) return rows;
	rows.push({ kind: "empty", id: "empty", reason: "query" });
	if (scope.length > 0) {
		rows.push({ kind: "widen", id: "widen:scope", widen: "scope" });
	}
	if (dropActive && canWiden) {
		rows.push({ kind: "widen", id: "widen:compatible", widen: "compatible" });
	}
	return rows;
}
