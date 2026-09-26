import { createId } from "@paralleldrive/cuid2";
import { upsertLayerCommand } from "./command/generic-command";
import { type GroupSuggestion, evaluateGroupCandidate } from "./flow-grouping";
import { type IBoard, ILayerType } from "./schema/flow/board";
import type { IGenericCommand } from "./schema/flow/board/commands/generic-command";

interface GroupSuggestionCommandInput {
	board: IBoard;
	suggestion: GroupSuggestion;
	currentLayer?: string;
}

/** Recheck the preview against the current graph before changing layer ownership. */
export function buildGroupSuggestionCommand({
	board,
	suggestion,
	currentLayer,
}: GroupSuggestionCommandInput): IGenericCommand | undefined {
	const parent = currentLayer || undefined;
	if (parent && !board.layers[parent]) return undefined;
	const current = evaluateGroupCandidate(
		{ board, currentLayer: parent },
		suggestion.memberIds,
	);
	if (!current || current.id !== suggestion.id) return undefined;
	const members = [...current.memberIds].sort();
	const previewMembers = [...suggestion.memberIds].sort();
	if (
		members.length !== previewMembers.length ||
		members.some((id, index) => id !== previewMembers[index])
	)
		return undefined;

	return upsertLayerCommand({
		layer: {
			id: createId(),
			name: suggestion.label.trim() || "Group",
			type: ILayerType.Collapsed,
			parent_id: parent,
			coordinates: [current.anchor.x, current.anchor.y, 0],
			nodes: {},
			pins: {},
			comments: {},
			variables: {},
		},
		node_ids: members,
		current_layer: parent,
	});
}
