import {
	type IBoard,
	ILayerType,
} from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type { WorkflowFocusTarget, WorkflowNodeDescriptor } from "./types";

export function describeWorkflowNodes(board: IBoard): WorkflowNodeDescriptor[] {
	const nodes = Object.values(board.nodes).map((node) => ({
		id: node.id,
		kind: "node" as const,
		name: node.name,
		friendlyName: node.friendly_name || undefined,
		layer: node.layer || undefined,
	}));
	const layers = Object.values(board.layers).map((layer) => ({
		id: layer.id,
		kind: "layer" as const,
		name: layer.name,
		layer: layer.parent_id || undefined,
	}));
	return [...nodes, ...layers].sort(
		(left, right) =>
			left.kind.localeCompare(right.kind) ||
			left.name.localeCompare(right.name) ||
			left.id.localeCompare(right.id),
	);
}

function stripAnchorSyntax(value: string): string {
	const trimmed = value.trim();
	const anchor = trimmed.match(/^(?:\/\/)?@(?:n|l):(.+)$/);
	const anchoredId = anchor?.[1];
	if (anchoredId) return anchoredId.trim();
	return trimmed;
}

function targetFromDescriptor(
	descriptor: WorkflowNodeDescriptor,
	matchedBy: WorkflowFocusTarget["matchedBy"],
): WorkflowFocusTarget {
	return {
		id: descriptor.id,
		kind: descriptor.kind,
		label: descriptor.friendlyName ?? descriptor.name,
		matchedBy,
	};
}

function uniqueMatch(
	query: string,
	matches: WorkflowNodeDescriptor[],
	matchedBy: WorkflowFocusTarget["matchedBy"],
): WorkflowFocusTarget | undefined {
	if (matches.length === 0) return undefined;
	const [match] = matches;
	if (matches.length === 1 && match) {
		return targetFromDescriptor(match, matchedBy);
	}
	const summary = matches
		.map(
			(match) =>
				`${match.id} (${match.friendlyName ?? match.name}${match.layer ? `, layer ${match.layer}` : ""})`,
		)
		.join(", ");
	throw new Error(
		`Focus selector ${JSON.stringify(query)} is ambiguous: ${summary}. Use an exact id or FlowScript anchor.`,
	);
}

export function resolveWorkflowFocus(
	board: IBoard,
	selector: string,
): WorkflowFocusTarget {
	const query = stripAnchorSyntax(selector);
	if (!query) throw new Error("--focus-node cannot be empty.");
	const descriptors = describeWorkflowNodes(board);

	const id = descriptors.find((descriptor) => descriptor.id === query);
	if (id) {
		return targetFromDescriptor(
			id,
			selector.trim().startsWith("@") || selector.trim().startsWith("//@")
				? "anchor"
				: "id",
		);
	}

	const exactName = uniqueMatch(
		selector,
		descriptors.filter(
			(descriptor) =>
				descriptor.name === query || descriptor.friendlyName === query,
		),
		"name",
	);
	if (exactName) return exactName;

	const folded = query.toLocaleLowerCase();
	const foldedMatch = uniqueMatch(
		selector,
		descriptors.filter(
			(descriptor) =>
				descriptor.id.toLocaleLowerCase() === folded ||
				descriptor.name.toLocaleLowerCase() === folded ||
				descriptor.friendlyName?.toLocaleLowerCase() === folded,
		),
		"name",
	);
	if (foldedMatch) return foldedMatch;

	const available = descriptors
		.slice(0, 12)
		.map((descriptor) => descriptor.friendlyName ?? descriptor.name)
		.join(", ");
	throw new Error(
		`No workflow node or layer matches ${JSON.stringify(selector)}.${available ? ` Available names include: ${available}.` : ""} Run with --list-nodes for exact ids.`,
	);
}

/** The DOM node that proves Studio finished navigating to a focus target. */
export function workflowFocusSentinelId(
	board: IBoard,
	target: WorkflowFocusTarget,
): string {
	const layer = target.kind === "layer" ? board.layers[target.id] : undefined;
	// Function layers open as canvases and are not themselves drawn there. Studio always renders
	// the function's input boundary, even when it has no pins, so it is the reliable load sentinel.
	if (layer?.type === ILayerType.Function) return `${target.id}-input`;
	return target.id;
}

/** Open a stable function/layer when Studio's root canvas would otherwise be empty. */
export function defaultWorkflowFocus(
	board: IBoard,
): WorkflowFocusTarget | undefined {
	const hasRootNode = Object.values(board.nodes).some(
		(node) => !node.layer || node.layer === "",
	);
	const hasRootLayer = Object.values(board.layers).some(
		(layer) =>
			layer.type !== ILayerType.Function &&
			(!layer.parent_id || layer.parent_id === ""),
	);
	const hasRootComment = Object.values(board.comments ?? {}).some(
		(comment) => !comment.layer || comment.layer === "",
	);
	if (hasRootNode || hasRootLayer || hasRootComment) return undefined;

	const layer = Object.values(board.layers).sort(
		(left, right) =>
			(left.type === ILayerType.Function ? 0 : 1) -
				(right.type === ILayerType.Function ? 0 : 1) ||
			left.name.localeCompare(right.name) ||
			left.id.localeCompare(right.id),
	)[0];
	if (!layer) return undefined;
	return {
		id: layer.id,
		kind: "layer",
		label: layer.name,
		matchedBy: "default",
	};
}
