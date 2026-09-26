import type { INode, IPin } from "../schema/flow/node";
import type { AutoLayoutInput } from "./types";
export interface AutoRerouteChain {
	from: string;
	to: string;
	fromPin: string;
	toPin: string;
	/** Original nodes in wire order, retained for reuse and undo. */
	nodes: INode[];
}
/** Treat generated, unbranched data reroutes as wire geometry during placement. */
export function normalizeAutoReroutes(input: AutoLayoutInput): {
	input: AutoLayoutInput;
	chains: AutoRerouteChain[];
} {
	const owners = new Map<string, string>();
	const pins = new Map<string, IPin>();
	const addPins = (id: string, values: Record<string, IPin>) => {
		for (const pin of Object.values(values)) {
			owners.set(pin.id, id);
			pins.set(pin.id, pin);
		}
	};
	for (const node of input.layerNodes) addPins(node.id, node.pins);
	for (const entity of input.layerEntities) {
		addPins(entity.id, input.boardLayers?.[entity.id]?.pins ?? {});
	}
	const incoming = new Map<string, Set<string>>();
	const outgoing = new Map<string, Set<string>>();
	const link = (from: string, to: string) => {
		const targets = outgoing.get(from) ?? new Set<string>();
		targets.add(to);
		outgoing.set(from, targets);
		const sources = incoming.get(to) ?? new Set<string>();
		sources.add(from);
		incoming.set(to, sources);
	};
	for (const pin of pins.values()) {
		if (pin.pin_type === "Output") {
			for (const target of pin.connected_to) link(pin.id, target);
		} else {
			for (const source of pin.depends_on) link(source, pin.id);
		}
	}
	const referenced = new Set(
		input.layerNodes.flatMap((node) => node.fn_refs?.fn_refs ?? []),
	);
	const candidates = new Map<
		string,
		{
			node: INode;
			input: IPin;
			output: IPin;
		}
	>();
	for (const node of input.layerNodes) {
		if (
			node.name !== "reroute" ||
			node.auto_reroute !== true ||
			node.wasm ||
			referenced.has(node.id)
		)
			continue;
		const values = Object.values(node.pins);
		const routeIn = values.find(
			(pin) => pin.name === "route_in" && pin.pin_type === "Input",
		);
		const routeOut = values.find(
			(pin) => pin.name === "route_out" && pin.pin_type === "Output",
		);
		if (values.length !== 2 || !routeIn || !routeOut) continue;
		if (routeIn.data_type === "Execution" || routeOut.data_type === "Execution")
			continue;
		if (routeIn.connected_to.length || routeOut.depends_on.length) continue;
		if (
			(incoming.get(routeIn.id)?.size ?? 0) !== 1 ||
			(outgoing.get(routeOut.id)?.size ?? 0) !== 1
		)
			continue;
		candidates.set(node.id, { node, input: routeIn, output: routeOut });
	}
	const chains: AutoRerouteChain[] = [];
	for (const id of [...candidates.keys()].sort()) {
		const start = required(candidates.get(id));
		const sourcePin = [...required(incoming.get(start.input.id))][0];
		const sourceId = owners.get(sourcePin);
		if (!sourceId || candidates.has(sourceId)) continue;
		const nodes: INode[] = [];
		const seen = new Set<string>();
		let current: typeof start | undefined = start;
		let targetPin: string | undefined;
		let targetId: string | undefined;
		while (current && !seen.has(current.node.id)) {
			seen.add(current.node.id);
			nodes.push(current.node);
			targetPin = [...required(outgoing.get(current.output.id))][0];
			targetId = owners.get(targetPin);
			current = targetId ? candidates.get(targetId) : undefined;
			if (current && current.input.id !== targetPin) break;
		}
		if (current || !targetId || !targetPin || sourceId === targetId) continue;
		// The two boundary boxes of an open layer share a runtime pin owner.
		// A direct same-owner wire is invalid, so keep their reroute topology.
		const boundaryIds = input.currentLayer
			? new Set([`${input.currentLayer}-input`, `${input.currentLayer}-return`])
			: undefined;
		if (boundaryIds?.has(sourceId) && boundaryIds.has(targetId)) continue;
		if (input.only && (!input.only.has(sourceId) || !input.only.has(targetId)))
			continue;
		if (
			pins.get(sourcePin)?.pin_type !== "Output" ||
			pins.get(targetPin)?.pin_type !== "Input"
		)
			continue;
		chains.push({
			from: sourceId,
			to: targetId,
			fromPin: sourcePin,
			toPin: targetPin,
			nodes,
		});
	}
	if (!chains.length) return { input, chains };
	const removed = new Set(
		chains.flatMap((chain) => chain.nodes.map((node) => node.id)),
	);
	const updates = new Map<string, IPin>();
	for (const chain of chains) {
		const first = required(
			Object.values(chain.nodes[0].pins).find((pin) => pin.name === "route_in"),
		);
		const last = required(
			Object.values(chain.nodes[chain.nodes.length - 1].pins).find(
				(pin) => pin.name === "route_out",
			),
		);
		const source = structuredClone(
			updates.get(chain.fromPin) ?? required(pins.get(chain.fromPin)),
		);
		const target = structuredClone(
			updates.get(chain.toPin) ?? required(pins.get(chain.toPin)),
		);
		source.connected_to = [
			...new Set([
				...source.connected_to.filter((id) => id !== first.id),
				chain.toPin,
			]),
		];
		target.depends_on = [
			...new Set([
				...target.depends_on.filter((id) => id !== last.id),
				chain.fromPin,
			]),
		];
		updates.set(source.id, source);
		updates.set(target.id, target);
	}
	const updatePins = (values: Record<string, IPin>) =>
		Object.fromEntries(
			Object.entries(values).map(([id, pin]) => [id, updates.get(id) ?? pin]),
		);
	const boardLayers = input.boardLayers ? { ...input.boardLayers } : undefined;
	if (boardLayers) {
		for (const entity of input.layerEntities) {
			const layer = boardLayers[entity.id];
			if (layer)
				boardLayers[entity.id] = { ...layer, pins: updatePins(layer.pins) };
		}
	}
	return {
		chains,
		input: {
			...input,
			layerNodes: input.layerNodes
				.filter((node) => !removed.has(node.id))
				.map((node) => ({ ...node, pins: updatePins(node.pins) })),
			boardLayers,
			only: input.only
				? new Set([...input.only].filter((id) => !removed.has(id)))
				: undefined,
			obstacles: input.obstacles?.filter(
				(box) => !box.id || !removed.has(box.id),
			),
		},
	};
}

function required<T>(value: T | null | undefined): T {
	if (value === undefined || value === null)
		throw new Error("Missing reroute graph value");
	return value;
}
