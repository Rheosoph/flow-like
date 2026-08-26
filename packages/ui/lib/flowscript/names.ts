import type { INode } from "../schema/flow/node";

/** One row of the generated `packages/ast/flow.d/names.json` snapshot, keyed by node type. */
export interface FlowScriptNodeNames {
	readonly qualified: string;
	readonly namespace: string;
	readonly alias: string;
	readonly flat: string;
	/** Receiver input pin name in method form; `null` when the node is static only. */
	readonly receiver: string | null;
	/** Method class of the receiver (`string`, `array`, a schema title); `null` when static only. */
	readonly class: string | null;
	readonly category: string;
}

export type FlowScriptNamesTable = Readonly<
	Record<string, FlowScriptNodeNames>
>;

export interface ResolvedFlowScriptNames {
	readonly flat: string;
	/** Namespace segments (`["ai", "ml"]`). */
	readonly namespace: readonly string[];
	readonly alias: string;
	/** `ai::ml::read`. */
	readonly qualified: string;
	/** Receiver input pin name; `undefined` when static only. */
	readonly receiver?: string;
	/** Method class when the snapshot knows it; callers derive it from the pin otherwise. */
	readonly class?: string;
}

let table: FlowScriptNamesTable | undefined;
let pending: Promise<FlowScriptNamesTable> | undefined;
const listeners = new Set<(names: FlowScriptNamesTable) => void>();

function isNamesTable(value: unknown): value is FlowScriptNamesTable {
	if (!value || typeof value !== "object") return false;
	const first = Object.values(value as Record<string, unknown>)[0];
	if (first === undefined) return true;
	if (!first || typeof first !== "object") return false;
	const row = first as Record<string, unknown>;
	return typeof row.qualified === "string" && typeof row.flat === "string";
}

/** The snapshot once it has loaded; `undefined` before (consumers fall back to flat names). */
export function getFlowScriptNamesTable(): FlowScriptNamesTable | undefined {
	return table;
}

/**
 * Loads the snapshot lazily (it is ~400 KB, so it stays out of the main bundle). Safe to call
 * repeatedly; resolves to the same table and notifies `onFlowScriptNamesTableLoaded` listeners once.
 */
export function loadFlowScriptNamesTable(): Promise<FlowScriptNamesTable> {
	if (table) return Promise.resolve(table);
	pending ??= import("../../../ast/flow.d/names.json").then((module) => {
		const loaded: unknown = module.default;
		table = isNamesTable(loaded) ? loaded : {};
		for (const listener of listeners) listener(table);
		listeners.clear();
		return table;
	});
	return pending;
}

/** Fires once when the snapshot arrives (immediately when it is already loaded). */
export function onFlowScriptNamesTableLoaded(
	listener: (names: FlowScriptNamesTable) => void,
): () => void {
	if (table) {
		listener(table);
		return () => {};
	}
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/** Splits a dotted (`ai.ml`) or path (`ai::ml`) namespace into its segments. */
export function namespaceSegments(namespace: string): string[] {
	return namespace
		.split(/::|\./)
		.map((segment) => segment.trim())
		.filter(Boolean);
}

/**
 * Explicit catalog fields win, then the generated snapshot by node type, else `undefined`
 * (flat-only). Names are never derived here: the Rust naming rule is the source of truth.
 */
export function resolveFlowScriptNames(
	node: Pick<INode, "name" | "namespace" | "alias" | "receiver">,
	names: FlowScriptNamesTable | undefined = table,
): ResolvedFlowScriptNames | undefined {
	const row = names?.[node.name];
	const explicitNamespace = node.namespace?.trim() || undefined;
	const explicitAlias = node.alias?.trim() || undefined;
	const namespace = namespaceSegments(
		explicitNamespace ?? row?.namespace ?? "",
	);
	const alias = explicitAlias ?? row?.alias;
	if (namespace.length === 0 || !alias) return undefined;

	const explicitReceiver = node.receiver;
	let receiver: string | undefined;
	let cls: string | undefined;
	if (explicitReceiver != null) {
		receiver = explicitReceiver.trim() || undefined;
		if (receiver && row?.receiver === receiver && row.class) cls = row.class;
	} else if (row?.receiver) {
		receiver = row.receiver;
		cls = row.class ?? undefined;
	}

	return {
		flat: row?.flat ?? "",
		namespace,
		alias,
		qualified: [...namespace, alias].join("::"),
		receiver,
		class: cls,
	};
}
