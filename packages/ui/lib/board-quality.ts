import type { TFunction } from "i18next";
import { SCORE_FLAG_THRESHOLD } from "./board-metrics";
import {
	type IBoard,
	type ILayer,
	ILayerType,
	type INode,
	type IPin,
	IPinType,
	type IVariable,
	IVariableType,
} from "./schema/flow/board";
import { parseUint8ArrayToJson } from "./uint8";

/**
 * Static linter over a live board. Pure and synchronous: one graph build, one
 * execution-reachability pass and one SCC pass feed every rule, so a full run
 * over a few thousand nodes stays in the low milliseconds and can follow every
 * board edit from a debounced effect.
 */

export type IQualityRule =
	| "dead-code"
	| "complexity"
	| "insecure-node"
	| "missing-input"
	| "no-return"
	| "cycle"
	| "hardcoded-secret"
	| "payment-readiness";

/** Panel order. Severity sorts above this. */
export const QUALITY_RULES: readonly IQualityRule[] = [
	"hardcoded-secret",
	"payment-readiness",
	"cycle",
	"no-return",
	"missing-input",
	"insecure-node",
	"dead-code",
	"complexity",
];

export type IQualitySeverity = "error" | "warning" | "info";

const SEVERITY_RANK: Record<IQualitySeverity, number> = {
	error: 0,
	warning: 1,
	info: 2,
};

export type IQualityTargetKind = "node" | "layer" | "variable";

export interface IQualityTarget {
	kind: IQualityTargetKind;
	id: string;
	name: string;
	/** The layer the target sits in; `null` at the board root. */
	layerId: string | null;
}

export type IQualityDetail =
	| { kind: "payment-readiness"; simulation: boolean }
	| { kind: "unreachable" }
	| { kind: "unused-result" }
	| { kind: "uncalled-function" }
	| { kind: "complex"; nodes: number; branches: number; depth: number }
	| { kind: "low-score"; category: "security" | "privacy"; score: number }
	| { kind: "wasm-network"; permissions: string[] }
	| { kind: "missing-input"; pin: string }
	| { kind: "entry-unwired" }
	| { kind: "exec-never-returns" }
	| { kind: "output-unassigned"; pin: string }
	| { kind: "no-outputs" }
	| { kind: "exec-cycle"; members: string[]; exit: boolean }
	| { kind: "data-cycle"; members: string[] }
	| { kind: "recursion"; functionName: string }
	| { kind: "sensitive-literal"; pin: string; connected: boolean }
	| { kind: "secret-like-literal"; pin: string }
	| { kind: "secret-pattern"; pin: string; pattern: string }
	| { kind: "variable-not-secret"; pattern?: string };

export interface IQualityFinding {
	/** Stable across runs while the underlying fact holds — keys UI rows and mark diffs. */
	id: string;
	rule: IQualityRule;
	severity: IQualitySeverity;
	target: IQualityTarget;
	detail: IQualityDetail;
}

export interface IQualityCounts {
	error: number;
	warning: number;
	info: number;
}

/** What one canvas element shows: its own findings plus everything nested below it. */
export interface IQualityMark {
	severity: IQualitySeverity;
	counts: IQualityCounts;
	findings: IQualityFinding[];
	/** Findings on nodes inside this layer, at any depth. */
	nested: number;
	/** Change detector for the store's structural sharing. */
	signature: string;
}

export interface IBoardQualityReport {
	findings: IQualityFinding[];
	counts: IQualityCounts;
	marks: Record<string, IQualityMark>;
	nodeCount: number;
	durationMs: number;
}

export const COMPLEXITY_LIMITS = {
	nodes: 40,
	branches: 10,
	depth: 40,
} as const;
export const COMPLEXITY_SEVERE = {
	nodes: 100,
	branches: 25,
	depth: 80,
} as const;

/** Nodes that can stop an execution cycle from re-triggering forever. */
const CYCLE_BREAKERS = new Set([
	"control_branch",
	"control_switch",
	"control_gate",
	"control_do_once",
	"control_do_n",
	"control_flip_flop",
	"control_for_each_with_break",
	"control_timeout",
	"control_while_loop",
]);

const CALL_FUNCTION_NODE = "control_call_function";
const FUNCTION_LAYER_PIN = "function_layer_id";

const NETWORK_PERMISSION = /^Network/;

interface ISecretPattern {
	id: string;
	regex: RegExp;
}

/** Token shapes with a recognizable prefix. Each is bounded so a longer word cannot match by accident. */
const SECRET_VALUE_PATTERNS: readonly ISecretPattern[] = [
	{ id: "private-key", regex: /-----BEGIN [A-Z ]*PRIVATE KEY-----/ },
	{ id: "openai", regex: /(?:^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{16,}/ },
	{ id: "aws", regex: /(?:^|[^A-Z0-9])AKIA[0-9A-Z]{16}(?![A-Z0-9])/ },
	{ id: "github", regex: /(?:^|[^A-Za-z0-9_])gh[pousr]_[A-Za-z0-9]{20,}/ },
	{ id: "gitlab", regex: /(?:^|[^A-Za-z0-9_-])glpat-[A-Za-z0-9_-]{20,}/ },
	{ id: "slack", regex: /(?:^|[^A-Za-z0-9_-])xox[abprs]-[A-Za-z0-9-]{10,}/ },
	{
		id: "google",
		regex: /(?:^|[^A-Za-z0-9_-])AIza[0-9A-Za-z_-]{35}(?![0-9A-Za-z_-])/,
	},
	{ id: "huggingface", regex: /(?:^|[^A-Za-z0-9_])hf_[A-Za-z0-9]{20,}/ },
	{
		id: "stripe",
		regex: /(?:^|[^A-Za-z0-9_])(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{10,}/,
	},
	{
		id: "sendgrid",
		regex: /(?:^|[^A-Za-z0-9_.-])SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}/,
	},
	{
		id: "telegram",
		regex: /(?:^|[^0-9])\d{8,10}:[A-Za-z0-9_-]{35}(?![A-Za-z0-9_-])/,
	},
	{
		id: "jwt",
		regex:
			/(?:^|[^A-Za-z0-9_.-])eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}/,
	},
	{ id: "bearer", regex: /\bBearer\s+[A-Za-z0-9._~+/=-]{20,}/i },
	{
		id: "url-credentials",
		regex: /\b[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+:[^\s/@]+@/i,
	},
];

const SECRET_WORDS = new Set([
	"secret",
	"secrets",
	"password",
	"passwd",
	"passphrase",
	"credential",
	"credentials",
	"bearer",
	"authorization",
]);

/** `token` is a secret unless the name is clearly about counting them. */
const TOKEN_COUNT_WORDS = new Set([
	"max",
	"min",
	"count",
	"limit",
	"length",
	"num",
	"number",
	"budget",
	"usage",
	"total",
	"input",
	"output",
	"per",
	"size",
]);

export function nameWords(name: string): string[] {
	return name
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.split(/[^A-Za-z0-9]+/)
		.filter(Boolean)
		.map((word) => word.toLowerCase());
}

/** Whether a pin or variable name reads as a secret carrier. */
export function looksLikeSecretName(name: string): boolean {
	const words = nameWords(name);
	if (words.length === 0) return false;
	const has = (word: string) => words.includes(word);
	if (words.some((word) => SECRET_WORDS.has(word))) return true;
	if (has("apikey") || (has("api") && has("key"))) return true;
	if (has("access") && has("key")) return true;
	if (has("private") && has("key")) return true;
	if (has("client") && has("secret")) return true;
	if (has("token") || has("tokens")) {
		return !words.some((word) => TOKEN_COUNT_WORDS.has(word));
	}
	return false;
}

export function matchSecretValue(value: string): string | undefined {
	if (value.length < 8) return undefined;
	return SECRET_VALUE_PATTERNS.find(({ regex }) => regex.test(value))?.id;
}

/** A literal that could stand in for a real secret: no whitespace, not a URL, not a placeholder. */
function plausibleSecretLiteral(value: string): boolean {
	const trimmed = value.trim();
	if (trimmed.length < 8 || /\s/.test(trimmed)) return false;
	if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)) return false;
	if (/^[<{[$][\s\S]*[>}\]]$/.test(trimmed)) return false;
	if (/^(?:your|my|the|a|an|insert|enter|paste|replace)[-_ ]/i.test(trimmed)) {
		return false;
	}
	if (/^(?:x+|\*+|\.+|-+|0+)$/i.test(trimmed)) return false;
	return true;
}

function literalValue(pin: { default_value?: number[] | null }): unknown {
	return parseUint8ArrayToJson(pin.default_value);
}

function carriesValue(value: unknown): boolean {
	return value !== undefined && value !== null;
}

function isExecPin(pin: IPin): boolean {
	return pin.data_type === IVariableType.Execution;
}

type PinOwner =
	| { kind: "node"; id: string }
	| { kind: "layer"; id: string; layer: ILayer };

/** What a pin chain ends at once relays are folded away. */
type Endpoint =
	| { kind: "node"; id: string }
	| { kind: "function-boundary"; layerId: string };

interface Graph {
	nodes: Map<string, INode>;
	layers: Map<string, ILayer>;
	pins: Map<string, IPin>;
	owners: Map<string, PinOwner>;
	/** Forward execution adjacency: node → nodes its exec outputs trigger. */
	execOut: Map<string, Set<string>>;
	impure: Set<string>;
}

function buildGraph(board: IBoard): Graph {
	const nodes = new Map<string, INode>();
	const layers = new Map<string, ILayer>();
	const pins = new Map<string, IPin>();
	const owners = new Map<string, PinOwner>();
	const impure = new Set<string>();

	for (const node of Object.values(board.nodes ?? {})) {
		if (!node?.id) continue;
		nodes.set(node.id, node);
		for (const pin of Object.values(node.pins ?? {})) {
			pins.set(pin.id, pin);
			owners.set(pin.id, { kind: "node", id: node.id });
			if (isExecPin(pin)) impure.add(node.id);
		}
	}
	for (const layer of Object.values(board.layers ?? {})) {
		if (!layer?.id) continue;
		layers.set(layer.id, layer);
		for (const pin of Object.values(layer.pins ?? {})) {
			pins.set(pin.id, pin);
			owners.set(pin.id, { kind: "layer", id: layer.id, layer });
		}
	}

	const graph: Graph = {
		nodes,
		layers,
		pins,
		owners,
		execOut: new Map(),
		impure,
	};

	for (const node of nodes.values()) {
		const targets = new Set<string>();
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.pin_type !== IPinType.Output || !isExecPin(pin)) continue;
			for (const endpoint of resolveForward(graph, pin.connected_to)) {
				if (endpoint.kind === "node") targets.add(endpoint.id);
			}
		}
		graph.execOut.set(node.id, targets);
	}
	return graph;
}

/**
 * Collapsed-layer boundary pins are relays: outside → layer pin → inside.
 * Function-layer boundary pins are the signature instead, so a chain ending
 * there is a parameter (backward) or a return (forward), never another node.
 */
function resolveForward(graph: Graph, pinIds: readonly string[]): Endpoint[] {
	const out: Endpoint[] = [];
	const seen = new Set<string>();
	const stack = [...pinIds];
	while (stack.length > 0) {
		const pinId = stack.pop() as string;
		if (seen.has(pinId)) continue;
		seen.add(pinId);
		const owner = graph.owners.get(pinId);
		if (!owner) continue;
		if (owner.kind === "node") {
			out.push({ kind: "node", id: owner.id });
			continue;
		}
		if (owner.layer.type === ILayerType.Function) {
			out.push({ kind: "function-boundary", layerId: owner.id });
			continue;
		}
		const pin = graph.pins.get(pinId);
		if (pin) stack.push(...pin.connected_to);
	}
	return out;
}

function resolveBackward(graph: Graph, pinIds: readonly string[]): Endpoint[] {
	const out: Endpoint[] = [];
	const seen = new Set<string>();
	const stack = [...pinIds];
	while (stack.length > 0) {
		const pinId = stack.pop() as string;
		if (seen.has(pinId)) continue;
		seen.add(pinId);
		const owner = graph.owners.get(pinId);
		if (!owner) continue;
		if (owner.kind === "node") {
			out.push({ kind: "node", id: owner.id });
			continue;
		}
		if (owner.layer.type === ILayerType.Function) {
			out.push({ kind: "function-boundary", layerId: owner.id });
			continue;
		}
		const pin = graph.pins.get(pinId);
		if (pin) stack.push(...pin.depends_on);
	}
	return out;
}

function dataSourceNodes(graph: Graph, node: INode): string[] {
	const sources: string[] = [];
	for (const pin of Object.values(node.pins ?? {})) {
		if (pin.pin_type !== IPinType.Input || isExecPin(pin)) continue;
		for (const endpoint of resolveBackward(graph, pin.depends_on)) {
			if (endpoint.kind === "node") sources.push(endpoint.id);
		}
	}
	return sources;
}

function nodeName(node: INode): string {
	return node.friendly_name?.trim() || node.name;
}

function nodeTarget(node: INode): IQualityTarget {
	return {
		kind: "node",
		id: node.id,
		name: nodeName(node),
		layerId: node.layer ?? null,
	};
}

function layerTarget(layer: ILayer): IQualityTarget {
	return {
		kind: "layer",
		id: layer.id,
		name: layer.name,
		layerId: layer.parent_id ?? null,
	};
}

function functionEntryNodes(graph: Graph, layer: ILayer): string[] {
	const entries: string[] = [];
	for (const pin of Object.values(layer.pins ?? {})) {
		if (pin.pin_type !== IPinType.Input || !isExecPin(pin)) continue;
		for (const endpoint of resolveForward(graph, pin.connected_to)) {
			if (endpoint.kind === "node") entries.push(endpoint.id);
		}
	}
	return entries;
}

function hasExecInput(node: INode): boolean {
	return Object.values(node.pins ?? {}).some(
		(pin) => pin.pin_type === IPinType.Input && isExecPin(pin),
	);
}

function isEntryNode(node: INode): boolean {
	return (
		node.start === true || node.event_callback === true || !hasExecInput(node)
	);
}

function bfs(graph: Graph, roots: Iterable<string>): Set<string> {
	const seen = new Set<string>();
	const queue: string[] = [];
	for (const root of roots) {
		if (!graph.nodes.has(root) || seen.has(root)) continue;
		seen.add(root);
		queue.push(root);
	}
	while (queue.length > 0) {
		const current = queue.pop() as string;
		for (const next of graph.execOut.get(current) ?? []) {
			if (seen.has(next)) continue;
			seen.add(next);
			queue.push(next);
		}
	}
	return seen;
}

interface Reachability {
	/** Impure nodes some entry point triggers. */
	reachable: Set<string>;
	/** Reachable impure nodes plus every pure node whose value they (transitively) read. */
	live: Set<string>;
}

function computeReachability(graph: Graph): Reachability {
	const roots: string[] = [];
	for (const node of graph.nodes.values()) {
		if (graph.impure.has(node.id) && isEntryNode(node)) roots.push(node.id);
	}
	for (const layer of graph.layers.values()) {
		if (layer.type !== ILayerType.Function) continue;
		roots.push(...functionEntryNodes(graph, layer));
	}
	const reachable = bfs(graph, roots);

	const live = new Set(reachable);
	const queue = [...reachable];
	for (const layer of graph.layers.values()) {
		if (layer.type !== ILayerType.Function) continue;
		for (const pin of Object.values(layer.pins ?? {})) {
			if (pin.pin_type !== IPinType.Output || isExecPin(pin)) continue;
			for (const endpoint of resolveBackward(graph, pin.depends_on)) {
				if (endpoint.kind !== "node" || live.has(endpoint.id)) continue;
				live.add(endpoint.id);
				queue.push(endpoint.id);
			}
		}
	}
	while (queue.length > 0) {
		const current = queue.pop() as string;
		const node = graph.nodes.get(current);
		if (!node) continue;
		for (const source of dataSourceNodes(graph, node)) {
			if (live.has(source) || graph.impure.has(source)) continue;
			live.add(source);
			queue.push(source);
		}
	}
	return { reachable, live };
}

function callTargets(graph: Graph): Map<string, string[]> {
	const calls = new Map<string, string[]>();
	for (const node of graph.nodes.values()) {
		if (node.name !== CALL_FUNCTION_NODE) continue;
		const pin = Object.values(node.pins ?? {}).find(
			(candidate) => candidate.name === FUNCTION_LAYER_PIN,
		);
		const layerId = pin ? literalValue(pin) : undefined;
		if (typeof layerId !== "string" || !layerId) continue;
		const list = calls.get(layerId) ?? [];
		list.push(node.id);
		calls.set(layerId, list);
	}
	return calls;
}

function ancestorLayers(graph: Graph, layerId: string | null | undefined) {
	const chain: string[] = [];
	let current = layerId ?? undefined;
	let guard = 0;
	while (current && guard < 64 && !chain.includes(current)) {
		chain.push(current);
		current = graph.layers.get(current)?.parent_id ?? undefined;
		guard += 1;
	}
	return chain;
}

class Findings {
	readonly list: IQualityFinding[] = [];

	push(
		rule: IQualityRule,
		severity: IQualitySeverity,
		target: IQualityTarget,
		detail: IQualityDetail,
		discriminator?: string,
	) {
		const suffix = discriminator ? `:${discriminator}` : "";
		this.list.push({
			id: `${rule}:${target.id}:${detail.kind}${suffix}`,
			rule,
			severity,
			target,
			detail,
		});
	}
}

function checkDeadCode(
	graph: Graph,
	{ reachable, live }: Reachability,
	calls: Map<string, string[]>,
	out: Findings,
) {
	for (const node of graph.nodes.values()) {
		if (node.name === "reroute") continue;
		if (graph.impure.has(node.id)) {
			if (!reachable.has(node.id)) {
				out.push("dead-code", "warning", nodeTarget(node), {
					kind: "unreachable",
				});
			}
			continue;
		}
		if (live.has(node.id)) continue;
		const outputs = Object.values(node.pins ?? {}).filter(
			(pin) => pin.pin_type === IPinType.Output,
		);
		if (outputs.length === 0) continue;
		// Only the tail of a dead chain is reported: a producer wired into
		// another dead node is covered by that node's own finding.
		const consumed = outputs.some(
			(pin) => resolveForward(graph, pin.connected_to).length > 0,
		);
		if (consumed) continue;
		out.push("dead-code", "info", nodeTarget(node), { kind: "unused-result" });
	}
	for (const layer of graph.layers.values()) {
		if (layer.type !== ILayerType.Function) continue;
		if ((calls.get(layer.id)?.length ?? 0) > 0) continue;
		out.push("dead-code", "info", layerTarget(layer), {
			kind: "uncalled-function",
		});
	}
}

function checkMissingInputs(
	graph: Graph,
	{ live }: Reachability,
	out: Findings,
) {
	for (const node of graph.nodes.values()) {
		if (!live.has(node.id) || node.name === "reroute") continue;
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.pin_type !== IPinType.Input || isExecPin(pin)) continue;
			if (carriesValue(literalValue(pin))) continue;
			if (resolveBackward(graph, pin.depends_on).length > 0) continue;
			out.push(
				"missing-input",
				"warning",
				nodeTarget(node),
				{ kind: "missing-input", pin: pin.friendly_name || pin.name },
				pin.id,
			);
		}
	}
}

function checkFunctions(graph: Graph, out: Findings) {
	for (const layer of graph.layers.values()) {
		if (layer.type !== ILayerType.Function) continue;
		const target = layerTarget(layer);
		const pins = Object.values(layer.pins ?? {});
		const execIn = pins.filter(
			(pin) => pin.pin_type === IPinType.Input && isExecPin(pin),
		);
		const execOut = pins.filter(
			(pin) => pin.pin_type === IPinType.Output && isExecPin(pin),
		);
		const dataOut = pins.filter(
			(pin) => pin.pin_type === IPinType.Output && !isExecPin(pin),
		);

		if (execIn.length > 0 && functionEntryNodes(graph, layer).length === 0) {
			out.push("no-return", "error", target, { kind: "entry-unwired" });
		}
		if (
			execOut.length > 0 &&
			!execOut.some((pin) => resolveBackward(graph, pin.depends_on).length > 0)
		) {
			out.push("no-return", "warning", target, { kind: "exec-never-returns" });
		}
		for (const pin of dataOut) {
			if (resolveBackward(graph, pin.depends_on).length > 0) continue;
			out.push(
				"no-return",
				"warning",
				target,
				{ kind: "output-unassigned", pin: pin.friendly_name || pin.name },
				pin.id,
			);
		}
		if (execIn.length === 0 && execOut.length === 0 && dataOut.length === 0) {
			out.push("no-return", "info", target, { kind: "no-outputs" });
		}
	}
}

/** Tarjan over `next`, restricted to `members`. Returns components of size > 1 or with a self-loop. */
function cyclicComponents(
	members: Iterable<string>,
	next: (id: string) => Iterable<string>,
): string[][] {
	const index = new Map<string, number>();
	const low = new Map<string, number>();
	const onStack = new Set<string>();
	const stack: string[] = [];
	const result: string[][] = [];
	let counter = 0;

	const visit = (start: string) => {
		const work: Array<{ id: string; iter: Iterator<string> }> = [];
		const enter = (id: string) => {
			index.set(id, counter);
			low.set(id, counter);
			counter += 1;
			stack.push(id);
			onStack.add(id);
			work.push({ id, iter: next(id)[Symbol.iterator]() });
		};
		enter(start);
		while (work.length > 0) {
			const frame = work[work.length - 1];
			const step = frame.iter.next();
			if (!step.done) {
				const target = step.value;
				if (!index.has(target)) {
					enter(target);
				} else if (onStack.has(target)) {
					low.set(
						frame.id,
						Math.min(low.get(frame.id) as number, index.get(target) as number),
					);
				}
				continue;
			}
			work.pop();
			if (work.length > 0) {
				const parent = work[work.length - 1].id;
				low.set(
					parent,
					Math.min(low.get(parent) as number, low.get(frame.id) as number),
				);
			}
			if (low.get(frame.id) !== index.get(frame.id)) continue;
			const component: string[] = [];
			let popped: string | undefined;
			do {
				popped = stack.pop();
				if (popped === undefined) break;
				onStack.delete(popped);
				component.push(popped);
			} while (popped !== frame.id);
			const selfLoop =
				component.length === 1 &&
				Array.from(next(component[0])).includes(component[0]);
			if (component.length > 1 || selfLoop) result.push(component);
		}
	};

	for (const member of members) {
		if (!index.has(member)) visit(member);
	}
	return result;
}

function checkCycles(
	graph: Graph,
	calls: Map<string, string[]>,
	out: Findings,
) {
	const execMembers = Array.from(graph.impure).filter((id) =>
		graph.nodes.has(id),
	);
	for (const component of cyclicComponents(
		execMembers,
		(id) => graph.execOut.get(id) ?? [],
	)) {
		const inside = new Set(component);
		const exit = component.some((id) => {
			const node = graph.nodes.get(id);
			if (node && CYCLE_BREAKERS.has(node.name)) return true;
			for (const target of graph.execOut.get(id) ?? []) {
				if (!inside.has(target)) return true;
			}
			return false;
		});
		const members = component
			.map((id) => graph.nodes.get(id))
			.filter((node): node is INode => Boolean(node));
		const names = members.map(nodeName);
		const anchor = members.slice().sort((a, b) => a.id.localeCompare(b.id))[0];
		if (!anchor) continue;
		out.push(
			"cycle",
			exit ? "warning" : "error",
			nodeTarget(anchor),
			{ kind: "exec-cycle", members: names, exit },
			component.slice().sort().join(","),
		);
	}

	const pureMembers = Array.from(graph.nodes.keys()).filter(
		(id) => !graph.impure.has(id),
	);
	const dataNext = new Map<string, string[]>();
	for (const id of pureMembers) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		dataNext.set(
			id,
			dataSourceNodes(graph, node).filter(
				(source) => !graph.impure.has(source),
			),
		);
	}
	for (const component of cyclicComponents(
		pureMembers,
		(id) => dataNext.get(id) ?? [],
	)) {
		const members = component
			.map((id) => graph.nodes.get(id))
			.filter((node): node is INode => Boolean(node));
		const anchor = members.slice().sort((a, b) => a.id.localeCompare(b.id))[0];
		if (!anchor) continue;
		out.push(
			"cycle",
			"error",
			nodeTarget(anchor),
			{ kind: "data-cycle", members: members.map(nodeName) },
			component.slice().sort().join(","),
		);
	}

	for (const [layerId, callers] of calls) {
		const layer = graph.layers.get(layerId);
		if (!layer) continue;
		for (const callerId of callers) {
			const caller = graph.nodes.get(callerId);
			if (!caller) continue;
			if (!ancestorLayers(graph, caller.layer).includes(layerId)) continue;
			out.push("cycle", "warning", nodeTarget(caller), {
				kind: "recursion",
				functionName: layer.name,
			});
		}
	}
}

function longestPath(
	region: Set<string>,
	next: (id: string) => Iterable<string>,
	roots: string[],
): number {
	const memo = new Map<string, number>();
	const onPath = new Set<string>();
	const depthOf = (start: string): number => {
		const work: Array<{ id: string; iter: Iterator<string>; best: number }> =
			[];
		const enter = (id: string) => {
			onPath.add(id);
			work.push({ id, iter: next(id)[Symbol.iterator](), best: 0 });
		};
		enter(start);
		while (work.length > 0) {
			const frame = work[work.length - 1];
			const step = frame.iter.next();
			if (!step.done) {
				const target = step.value;
				if (!region.has(target) || onPath.has(target)) continue;
				const known = memo.get(target);
				if (known !== undefined) {
					frame.best = Math.max(frame.best, known + 1);
					continue;
				}
				enter(target);
				continue;
			}
			work.pop();
			onPath.delete(frame.id);
			memo.set(frame.id, frame.best);
			const parent = work[work.length - 1];
			if (parent) parent.best = Math.max(parent.best, frame.best + 1);
		}
		return memo.get(start) ?? 0;
	};
	let longest = 0;
	for (const root of roots) {
		if (!region.has(root)) continue;
		longest = Math.max(longest, depthOf(root) + 1);
	}
	return longest;
}

function regionMetrics(graph: Graph, roots: string[]) {
	const region = bfs(graph, roots);
	const involved = new Set(region);
	const queue = [...region];
	while (queue.length > 0) {
		const node = graph.nodes.get(queue.pop() as string);
		if (!node) continue;
		for (const source of dataSourceNodes(graph, node)) {
			if (involved.has(source) || graph.impure.has(source)) continue;
			involved.add(source);
			queue.push(source);
		}
	}
	let branches = 0;
	for (const id of region) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		let wired = 0;
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.pin_type !== IPinType.Output || !isExecPin(pin)) continue;
			if (resolveForward(graph, pin.connected_to).length > 0) wired += 1;
		}
		branches += Math.max(0, wired - 1);
	}
	const depth = longestPath(region, (id) => graph.execOut.get(id) ?? [], roots);
	return { nodes: involved.size, branches, depth };
}

function complexitySeverity(metrics: {
	nodes: number;
	branches: number;
	depth: number;
}): IQualitySeverity | undefined {
	if (
		metrics.nodes > COMPLEXITY_SEVERE.nodes ||
		metrics.branches > COMPLEXITY_SEVERE.branches ||
		metrics.depth > COMPLEXITY_SEVERE.depth
	) {
		return "error";
	}
	if (
		metrics.nodes > COMPLEXITY_LIMITS.nodes ||
		metrics.branches > COMPLEXITY_LIMITS.branches ||
		metrics.depth > COMPLEXITY_LIMITS.depth
	) {
		return "warning";
	}
	return undefined;
}

function checkComplexity(graph: Graph, out: Findings) {
	for (const node of graph.nodes.values()) {
		if (node.start !== true) continue;
		const metrics = regionMetrics(graph, [node.id]);
		const severity = complexitySeverity(metrics);
		if (!severity) continue;
		out.push("complexity", severity, nodeTarget(node), {
			kind: "complex",
			...metrics,
		});
	}
	for (const layer of graph.layers.values()) {
		if (layer.type !== ILayerType.Function) continue;
		const metrics = regionMetrics(graph, functionEntryNodes(graph, layer));
		const severity = complexitySeverity(metrics);
		if (!severity) continue;
		out.push("complexity", severity, layerTarget(layer), {
			kind: "complex",
			...metrics,
		});
	}
}

function checkPaymentReadiness(graph: Graph, out: Findings) {
	for (const node of graph.nodes.values()) {
		if (node.name !== "request_payment") continue;
		const pin = Object.values(node.pins).find(
			(pin) => pin.name === "simulation",
		);
		let simulation = false;
		if (pin?.default_value && pin.depends_on.length === 0) {
			try {
				const value = parseUint8ArrayToJson(pin.default_value);
				simulation = typeof value === "string" && value.length > 0;
			} catch {
				// An invalid pin value is checked when the node executes.
			}
		}
		out.push(
			"payment-readiness",
			simulation ? "info" : "warning",
			nodeTarget(node),
			{
				kind: "payment-readiness",
				simulation,
			},
		);
	}
}

function checkInsecureNodes(graph: Graph, out: Findings) {
	for (const node of graph.nodes.values()) {
		if (node.name === "reroute") continue;
		const target = nodeTarget(node);
		for (const category of ["security", "privacy"] as const) {
			const score = node.scores?.[category];
			if (typeof score !== "number" || score >= SCORE_FLAG_THRESHOLD) continue;
			out.push(
				"insecure-node",
				score <= 1 ? "error" : "warning",
				target,
				{ kind: "low-score", category, score },
				category,
			);
		}
		const permissions = (node.wasm?.permissions ?? []).filter((permission) =>
			NETWORK_PERMISSION.test(permission),
		);
		if (permissions.length > 0) {
			out.push("insecure-node", "info", target, {
				kind: "wasm-network",
				permissions,
			});
		}
	}
}

function checkSecrets(graph: Graph, board: IBoard, out: Findings) {
	for (const node of graph.nodes.values()) {
		const target = nodeTarget(node);
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.pin_type !== IPinType.Input || isExecPin(pin)) continue;
			const value = literalValue(pin);
			if (typeof value !== "string" || value.trim() === "") continue;
			const pinName = pin.friendly_name || pin.name;
			const connected = resolveBackward(graph, pin.depends_on).length > 0;
			if (pin.options?.sensitive) {
				out.push(
					"hardcoded-secret",
					"error",
					target,
					{ kind: "sensitive-literal", pin: pinName, connected },
					pin.id,
				);
				continue;
			}
			const pattern = matchSecretValue(value);
			if (pattern) {
				out.push(
					"hardcoded-secret",
					"error",
					target,
					{ kind: "secret-pattern", pin: pinName, pattern },
					pin.id,
				);
				continue;
			}
			if (
				pin.data_type === IVariableType.String &&
				!connected &&
				looksLikeSecretName(pin.name) &&
				plausibleSecretLiteral(value)
			) {
				out.push(
					"hardcoded-secret",
					"warning",
					target,
					{ kind: "secret-like-literal", pin: pinName },
					pin.id,
				);
			}
		}
	}

	const variables: Array<{ variable: IVariable; layerId: string | null }> = [];
	for (const variable of Object.values(board.variables ?? {})) {
		variables.push({ variable, layerId: null });
	}
	for (const layer of graph.layers.values()) {
		for (const variable of Object.values(layer.variables ?? {})) {
			variables.push({ variable, layerId: layer.id });
		}
	}
	for (const { variable, layerId } of variables) {
		if (variable.secret) continue;
		const value = literalValue(variable);
		if (typeof value !== "string" || value.trim() === "") continue;
		const target: IQualityTarget = {
			kind: "variable",
			id: variable.id,
			name: variable.name,
			layerId,
		};
		const pattern = matchSecretValue(value);
		if (pattern) {
			out.push("hardcoded-secret", "error", target, {
				kind: "variable-not-secret",
				pattern,
			});
			continue;
		}
		if (looksLikeSecretName(variable.name) && plausibleSecretLiteral(value)) {
			out.push("hardcoded-secret", "warning", target, {
				kind: "variable-not-secret",
			});
		}
	}
}

function compareFindings(a: IQualityFinding, b: IQualityFinding): number {
	return (
		SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity] ||
		QUALITY_RULES.indexOf(a.rule) - QUALITY_RULES.indexOf(b.rule) ||
		a.target.name.localeCompare(b.target.name) ||
		a.id.localeCompare(b.id)
	);
}

function emptyCounts(): IQualityCounts {
	return { error: 0, warning: 0, info: 0 };
}

function worstSeverity(counts: IQualityCounts): IQualitySeverity {
	if (counts.error > 0) return "error";
	if (counts.warning > 0) return "warning";
	return "info";
}

function buildMarks(
	graph: Graph,
	findings: IQualityFinding[],
): Record<string, IQualityMark> {
	const own = new Map<string, IQualityFinding[]>();
	const nested = new Map<string, IQualityCounts>();
	for (const finding of findings) {
		if (finding.target.kind === "variable") continue;
		const list = own.get(finding.target.id) ?? [];
		list.push(finding);
		own.set(finding.target.id, list);
		for (const layerId of ancestorLayers(graph, finding.target.layerId)) {
			const counts = nested.get(layerId) ?? emptyCounts();
			counts[finding.severity] += 1;
			nested.set(layerId, counts);
		}
	}
	const marks: Record<string, IQualityMark> = {};
	const ids = new Set([...own.keys(), ...nested.keys()]);
	for (const id of ids) {
		const list = own.get(id) ?? [];
		const counts = emptyCounts();
		for (const finding of list) counts[finding.severity] += 1;
		const below = nested.get(id);
		const merged: IQualityCounts = {
			error: counts.error + (below?.error ?? 0),
			warning: counts.warning + (below?.warning ?? 0),
			info: counts.info + (below?.info ?? 0),
		};
		const nestedTotal = below ? below.error + below.warning + below.info : 0;
		marks[id] = {
			severity: worstSeverity(merged),
			counts: merged,
			findings: list,
			nested: nestedTotal,
			signature: `${list.map((finding) => `${finding.id}@${finding.severity}`).join("|")}#${merged.error}/${merged.warning}/${merged.info}`,
		};
	}
	return marks;
}

export function analyzeBoardQuality(board: IBoard): IBoardQualityReport {
	const startedAt =
		typeof performance !== "undefined" ? performance.now() : Date.now();
	const graph = buildGraph(board);
	const reachability = computeReachability(graph);
	const calls = callTargets(graph);
	const out = new Findings();

	checkSecrets(graph, board, out);
	checkCycles(graph, calls, out);
	checkFunctions(graph, out);
	checkMissingInputs(graph, reachability, out);
	checkInsecureNodes(graph, out);
	checkPaymentReadiness(graph, out);
	checkDeadCode(graph, reachability, calls, out);
	checkComplexity(graph, out);

	const findings = out.list.sort(compareFindings);
	const counts = emptyCounts();
	for (const finding of findings) counts[finding.severity] += 1;
	const endedAt =
		typeof performance !== "undefined" ? performance.now() : Date.now();
	return {
		findings,
		counts,
		marks: buildMarks(graph, findings),
		nodeCount: graph.nodes.size,
		durationMs: Math.max(0, endedAt - startedAt),
	};
}

export function emptyQualityReport(): IBoardQualityReport {
	return {
		findings: [],
		counts: emptyCounts(),
		marks: {},
		nodeCount: 0,
		durationMs: 0,
	};
}

/** The `flow`-namespaced `t` from `useTranslation("flow")`. */
export type QualityTranslate = TFunction<"flow">;

export function qualityRuleLabel(
	rule: IQualityRule,
	t: QualityTranslate,
): string {
	switch (rule) {
		case "payment-readiness":
			return t("qualityRulePaymentReadiness", "Payment readiness");
		case "dead-code":
			return t("qualityRuleDeadCode", "Dead code");
		case "complexity":
			return t("qualityRuleComplexity", "Too complex");
		case "insecure-node":
			return t("qualityRuleInsecureNode", "Potentially insecure nodes");
		case "missing-input":
			return t("qualityRuleMissingInput", "Missing required input");
		case "no-return":
			return t("qualityRuleNoReturn", "Function without return");
		case "cycle":
			return t("qualityRuleCycle", "Potential infinite cycle");
		case "hardcoded-secret":
			return t("qualityRuleHardcodedSecret", "Hard-coded secret");
	}
}

/** One line per finding; the panel prefixes the target name itself. */
export function describeQualityFinding(
	finding: IQualityFinding,
	t: QualityTranslate,
): string {
	const detail = finding.detail;
	switch (detail.kind) {
		case "payment-readiness":
			return detail.simulation
				? t(
						"qualityPaymentSimulation",
						"Payment simulation runs only in local Board Test. Clear Board Test Result before requesting real payments.",
					)
				: t(
						"qualityPaymentReadiness",
						"Real payments require a signed-in payer in an attended remote stream and an app owner who enabled payments and completed onboarding. Board analysis cannot verify account readiness.",
					);
		case "unreachable":
			return t(
				"qualityUnreachable",
				"No entry point reaches this node, so it never runs.",
			);
		case "unused-result":
			return t(
				"qualityUnusedResult",
				"Its result is not read by anything that runs.",
			);
		case "uncalled-function":
			return t("qualityUncalledFunction", "No Call Function node calls it.");
		case "complex":
			return t(
				"qualityComplex",
				"{{nodes}} nodes, {{branches}} branches, {{depth}} deep. Split it into functions.",
				detail,
			);
		case "low-score":
			return detail.category === "security"
				? t("qualityLowSecurity", "Security score {{score}}/10.", detail)
				: t("qualityLowPrivacy", "Privacy score {{score}}/10.", detail);
		case "wasm-network":
			return t(
				"qualityWasmNetwork",
				"Sandboxed node with network access: {{permissions}}.",
				{ permissions: detail.permissions.join(", ") },
			);
		case "missing-input":
			return t(
				"qualityMissingInput",
				"“{{pin}}” has no connection and no default value.",
				detail,
			);
		case "entry-unwired":
			return t(
				"qualityEntryUnwired",
				"Nothing is connected to the execution input, so calling it fails.",
			);
		case "exec-never-returns":
			return t(
				"qualityExecNeverReturns",
				"No path reaches the execution output, so callers never continue.",
			);
		case "output-unassigned":
			return t(
				"qualityOutputUnassigned",
				"Output “{{pin}}” is never assigned.",
				detail,
			);
		case "no-outputs":
			return t("qualityNoOutputs", "This function produces no outputs.");
		case "exec-cycle":
			return detail.exit
				? t(
						"qualityExecCycleWithExit",
						"Execution loops back through {{members}}. Make sure the exit condition is reached.",
						{ members: detail.members.join(" → ") },
					)
				: t(
						"qualityExecCycleNoExit",
						"Execution loops back through {{members}} with no way out.",
						{ members: detail.members.join(" → ") },
					);
		case "data-cycle":
			return t(
				"qualityDataCycle",
				"Circular data dependency between {{members}}.",
				{ members: detail.members.join(", ") },
			);
		case "recursion":
			return t(
				"qualityRecursion",
				"Calls “{{functionName}}” from inside itself; guard it with a base case.",
				detail,
			);
		case "sensitive-literal":
			return detail.connected
				? t(
						"qualitySensitiveLiteralConnected",
						"Sensitive input “{{pin}}” still stores a literal value on the board.",
						detail,
					)
				: t(
						"qualitySensitiveLiteral",
						"Sensitive input “{{pin}}” holds a literal value. Use a secret variable.",
						detail,
					);
		case "secret-like-literal":
			return t(
				"qualitySecretLikeLiteral",
				"“{{pin}}” looks like a credential and holds a literal value.",
				detail,
			);
		case "secret-pattern":
			return t(
				"qualitySecretPattern",
				"“{{pin}}” contains what looks like a {{pattern}} credential.",
				detail,
			);
		case "variable-not-secret":
			return detail.pattern
				? t(
						"qualityVariableSecretPattern",
						"Default value looks like a {{pattern}} credential but the variable is not marked secret.",
						detail,
					)
				: t(
						"qualityVariableNotSecret",
						"Looks like a credential but is not marked secret.",
					);
	}
}
