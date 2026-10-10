import { expect, test } from "bun:test";
import { prepareCommentSave } from "./flow-comment-edit";
import { type IComment, ICommentType } from "./schema/flow/board";

const current: IComment = {
	id: "comment",
	content: "new server text",
	comment_type: ICommentType.Text,
	coordinates: [1, 2, 0],
	timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
};

test("closing an untouched draft never overwrites a newer comment", () => {
	expect(prepareCommentSave(current, "old text", "old text")).toBeUndefined();
});
test("a concurrent content edit preserves the draft by rejecting save", () => {
	expect(() => prepareCommentSave(current, "old text", "my edit")).toThrow(
		"changed while you were editing",
	);
});
test("saving content keeps the latest position and color", () => {
	const next = prepareCommentSave(
		{ ...current, color: "blue", coordinates: [5, 7, 0] },
		current.content,
		"my edit",
	);
	expect(next).toMatchObject({
		content: "my edit",
		color: "blue",
		coordinates: [5, 7, 0],
	});
});
