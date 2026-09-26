import { findDropTargetPin } from "../flow-board-helpers";
import { doPinsMatch } from "../flow-board-utils";
import { visiblePinsOf } from "../flow-layout/measure";
import { type INode, type IPin, IPinType } from "../schema/flow/node";
import { GHOST_ALTERNATIVES, type SuggestionResult } from "./types";

export interface GhostAnchor {
	node: INode;
	pin: IPin;
}

export interface ResolvedGhost {
	/** The catalog template. Never mutate it; clone before placing. */
	node: INode;
	/** Pin of `node` the ghost wire lands on, opposite in direction to the anchor pin. */
	targetPinName: string;
	probability: number;
	/** `(1 - pUnconnected) · probability`. */
	confidence: number;
}

const oppositeOf = (pin: IPin) =>
	pin.pin_type === IPinType.Input ? IPinType.Output : IPinType.Input;

/**
 * `doPinsMatch` is called without node arguments on purpose: with them it flips `pin_type` in
 * place for layer boundary nodes, which would corrupt the catalog template.
 */
function targetPinOf(
	node: INode,
	predicted: string | null,
	anchorPin: IPin,
	refs: Record<string, string>,
): IPin | undefined {
	const direction = oppositeOf(anchorPin);
	const pins = visiblePinsOf(node)
		.filter((pin) => pin.pin_type === direction)
		.sort((a, b) => a.index - b.index);
	const fits = (pin: IPin | undefined): pin is IPin =>
		pin !== undefined && doPinsMatch(anchorPin, pin, refs);

	const named = predicted
		? pins.find((pin) => pin.name === predicted)
		: undefined;
	if (fits(named)) return named;

	const dropped = findDropTargetPin(
		Object.fromEntries(pins.map((pin) => [pin.id, pin])),
		anchorPin,
		refs,
	);
	if (fits(dropped)) return dropped;

	return pins.find(fits);
}

/**
 * Turns ranked node types into ghosts that can actually be wired to `anchor.pin`, best first.
 * Candidates missing from the catalog or without a type-compatible pin are skipped.
 */
export function resolveGhosts(
	result: SuggestionResult,
	anchor: GhostAnchor,
	catalogByName: ReadonlyMap<string, INode>,
	refs: Record<string, string>,
): ResolvedGhost[] {
	const ghosts: ResolvedGhost[] = [];
	const seen = new Set<string>();
	const connectable = 1 - result.pUnconnected;
	for (const candidate of result.candidates) {
		if (ghosts.length >= GHOST_ALTERNATIVES) break;
		if (seen.has(candidate.type)) continue;
		const node = catalogByName.get(candidate.type);
		if (!node) continue;
		const target = targetPinOf(node, candidate.pin, anchor.pin, refs);
		if (!target) continue;
		seen.add(candidate.type);
		ghosts.push({
			node,
			targetPinName: target.name,
			probability: candidate.probability,
			confidence: connectable * candidate.probability,
		});
	}
	return ghosts;
}
