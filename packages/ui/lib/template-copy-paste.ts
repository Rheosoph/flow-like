import { copyPasteCommand } from "./command/generic-command";
import type { IBoard } from "./schema/flow/board";
import type { ICopyPaste } from "./schema/flow/board/commands/copy-paste";
import type { IGenericCommand } from "./schema/flow/board/commands/generic-command";

const INTERNAL_BOARD_REF_PREFIX = "__flow_like_internal_v1/";

/**
 * Builds the command used by the empty-board template selector.
 *
 * Template pin schemas and descriptions may be compact keys into `board.refs`, so the ref table is
 * part of the copied graph rather than optional metadata. Keeping this builder separate from the
 * component makes that serialization contract easy to regression-test.
 */
export function buildTemplateCopyPasteCommand(
	template: IBoard,
	currentLayer?: string,
): IGenericCommand & ICopyPaste {
	const publicRefs = Object.fromEntries(
		Object.entries(template.refs ?? {}).filter(
			([key]) => !key.startsWith(INTERNAL_BOARD_REF_PREFIX),
		),
	);

	return copyPasteCommand({
		original_nodes: Object.values(template.nodes),
		original_comments: Object.values(template.comments),
		original_layers: Object.values(template.layers),
		original_variables: Object.values(template.variables),
		original_refs: publicRefs,
		new_nodes: [],
		new_comments: [],
		new_layers: [],
		current_layer: currentLayer,
		offset: [100, 100, 0],
		old_mouse: undefined,
	} as ICopyPaste) as IGenericCommand & ICopyPaste;
}
