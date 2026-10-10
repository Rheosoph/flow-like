import type { IComment } from "./schema/flow/board";

export function prepareCommentSave(
	current: IComment,
	baseContent: string,
	content: string,
): IComment | undefined {
	if (content === baseContent) return undefined;
	if (current.content !== baseContent) {
		throw new Error(
			"This comment changed while you were editing. Copy your draft or reload the latest comment before saving.",
		);
	}
	return { ...current, content };
}
