import { owningModuleId } from "./layer-to-function";
import { type ILayer, ILayerType } from "./schema/flow/board";

/** Bounds every parent walk, so a damaged `parent_id` chain degrades instead of looping. */
const MAX_MODULE_DEPTH = 40;

/** Extension every module wears in the tab strip and in the FlowScript panel. */
export const MODULE_FILE_EXTENSION = ".flow";

/** The root scope. "main" is not a layer — it is the board with no layer open. */
export const MAIN_FILE_LABEL = `main${MODULE_FILE_EXTENSION}`;

/**
 * File identity of the root scope. Every other file id is a module layer id, so the renderer,
 * the FlowScript panel and the apply contract can key on one string per file.
 */
export const MAIN_FILE_ID = "main";

/** The file a module id addresses; `null`/undefined is the root. */
export function moduleFileId(moduleId: string | null | undefined): string {
	return moduleId ?? MAIN_FILE_ID;
}

/** The module layer a file id addresses; `undefined` for `main`. */
export function fileModuleId(fileId: string | undefined): string | undefined {
	return !fileId || fileId === MAIN_FILE_ID ? undefined : fileId;
}

/**
 * Words a module may not be named after: a module name becomes a FlowScript namespace
 * segment, and a segment that lexes as a keyword makes the file unparseable.
 */
export const FLOWSCRIPT_KEYWORDS: readonly string[] = [
	"function",
	"const",
	"let",
	"use",
	"interface",
	"module",
	"if",
	"else",
	"for",
	"while",
	"return",
	"true",
	"false",
	"null",
];

/** A module layer as the tab strip and the FlowScript file list see it. */
export interface IBoardModule {
	id: string;
	/** The layer name as the user typed it. */
	name: string;
	/** `checkout/payments.flow` — the module's namespace path as a file name. */
	pathLabel: string;
}

/** Why a module name was rejected. Callers map these onto their own copy. */
export type IModuleNameError =
	| "empty"
	| "invalid_identifier"
	| "reserved"
	| "duplicate";

/**
 * camelCase a module name into its FlowScript namespace segment. Mirrors
 * `flow_like_ast::text::to_camel_case`, which is what lowering renders module paths
 * with — a divergence here would show one name on the tab and another in the script.
 */
export function toModuleIdent(name: string): string {
	let out = "";
	let upcomingUpper = false;
	let first = true;

	for (const char of name) {
		if (/\p{L}|\p{N}/u.test(char)) {
			if (first) {
				out += char.toLowerCase();
				first = false;
			} else if (upcomingUpper) {
				out += char.toUpperCase();
			} else {
				out += char;
			}
			upcomingUpper = false;
			continue;
		}
		// Any separator (`_`, `-`, `:`, `/`, space) upper-cases the next character.
		if (!first) upcomingUpper = true;
	}

	// A digit-leading identifier lexes as a number and breaks the whole document.
	return /^\d/.test(out) ? `_${out}` : out;
}

/**
 * The namespace path of a module, outermost first (`["checkout", "payments"]`). Mirrors
 * `lower::module_path`: the walk stops at the first ancestor that is not a module, since
 * a module only ever nests under another module. Empty when `moduleId` is not a module.
 */
export function modulePathSegments(
	layers: Record<string, ILayer> | undefined,
	moduleId: string,
): string[] {
	const segments: string[] = [];
	const seen = new Set<string>();
	let current: string | undefined = moduleId;

	while (current && segments.length < MAX_MODULE_DEPTH && !seen.has(current)) {
		const layer: ILayer | undefined = layers?.[current];
		if (!layer || layer.type !== ILayerType.Module) break;
		seen.add(current);
		segments.unshift(toModuleIdent(layer.name));
		current = layer.parent_id || undefined;
	}

	return segments;
}

/** The file label of a module (`checkout/payments.flow`). */
export function modulePathLabel(
	layers: Record<string, ILayer> | undefined,
	moduleId: string,
): string {
	const segments = modulePathSegments(layers, moduleId);
	if (segments.length === 0) return MAIN_FILE_LABEL;
	return `${segments.join("/")}${MODULE_FILE_EXTENSION}`;
}

/**
 * Every module on the board as a tab, ordered by the path the user reads. Modules are
 * virtual files: they never carry coordinates worth sorting by, so the label is the order.
 */
export function boardModules(
	layers: Record<string, ILayer> | undefined,
): IBoardModule[] {
	const modules: IBoardModule[] = [];

	for (const layer of Object.values(layers ?? {})) {
		if (layer.type !== ILayerType.Module) continue;
		modules.push({
			id: layer.id,
			name: layer.name,
			pathLabel: modulePathLabel(layers, layer.id),
		});
	}

	return modules.toSorted(
		(a, b) =>
			a.pathLabel.localeCompare(b.pathLabel) || a.id.localeCompare(b.id),
	);
}

/**
 * The module the canvas is currently inside, or null for main. A module open on the canvas
 * is its own context; anything else — a collapsed layer, a module-local function — belongs
 * to its nearest module ancestor.
 */
export function activeModuleId(
	layerPath: string | undefined,
	currentLayer: string | undefined,
	layers: Record<string, ILayer> | undefined,
): string | null {
	const layerId = currentLayer || layerPath?.split("/").pop();
	if (!layerId) return null;
	if (layers?.[layerId]?.type === ILayerType.Module) return layerId;
	return owningModuleId(layers, layerId);
}

/**
 * What the FlowScript editor needs to know about a board that spans several files. A file only
 * renders its own sections, so without this the client linter would flag every call that leaves
 * the file — a module path as an unknown catalog namespace, a root function as undeclared.
 */
export interface FlowScriptBoardScope {
	/** Namespace keys of every module (`checkout`, `checkout::payments`). */
	modules: readonly string[];
	/** Module namespace key → function names declared there; `""` is the root file. */
	functionsByModule: Readonly<Record<string, readonly string[]>>;
}

/**
 * The board's modules and its functions per module, named exactly as FlowScript renders them
 * (`lower::to_camel_case` for both the module segments and the function layer names).
 */
export function boardFlowScriptScope(
	layers: Record<string, ILayer> | undefined,
): FlowScriptBoardScope {
	const modules: string[] = [];
	const functionsByModule: Record<string, string[]> = {};

	for (const layer of Object.values(layers ?? {})) {
		if (layer.type === ILayerType.Module) {
			const key = modulePathSegments(layers, layer.id).join("::");
			if (key) modules.push(key);
			continue;
		}
		if (layer.type !== ILayerType.Function) continue;
		const owner = owningModuleId(layers, layer.id);
		const key = owner ? modulePathSegments(layers, owner).join("::") : "";
		functionsByModule[key] ??= [];
		functionsByModule[key].push(toModuleIdent(layer.name));
	}

	return { modules, functionsByModule };
}

/** Modules filed directly under `parentId` (null = a top-level module). */
export function siblingModules(
	layers: Record<string, ILayer> | undefined,
	parentId: string | null,
): ILayer[] {
	return Object.values(layers ?? {}).filter(
		(layer) =>
			layer.type === ILayerType.Module &&
			(layer.parent_id || null) === parentId,
	);
}

/**
 * Whether `name` can become a module under `parentId`. Modules resolve by their camelCased
 * name, case-insensitively, so two siblings that camelCase to the same identifier would make
 * every qualified call between them ambiguous — that is rejected here rather than at apply
 * time. `reservedRoots` are the names the surrounding FlowScript already owns (keywords,
 * catalog namespace roots).
 */
export function validateModuleName(
	name: string,
	layers: Record<string, ILayer> | undefined,
	parentId: string | null,
	reservedRoots: readonly string[] = FLOWSCRIPT_KEYWORDS,
	excludeId?: string,
): IModuleNameError | null {
	const trimmed = name.trim();
	if (!trimmed) return "empty";

	const ident = toModuleIdent(trimmed);
	if (!ident || !/^[\p{L}_$][\p{L}\p{N}_$]*$/u.test(ident)) {
		return "invalid_identifier";
	}

	const key = ident.toLowerCase();
	if (reservedRoots.some((reserved) => reserved.toLowerCase() === key)) {
		return "reserved";
	}

	const clash = siblingModules(layers, parentId).some(
		(layer) =>
			layer.id !== excludeId && toModuleIdent(layer.name).toLowerCase() === key,
	);
	return clash ? "duplicate" : null;
}
