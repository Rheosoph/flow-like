import { describe, expect, test } from "bun:test";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { approvalOf } from "./approval-entry";

const card = (approval: Record<string, unknown>) =>
	({
		id: "e1",
		kind: "message",
		envelope: {
			v: 1,
			enc: "none",
			body: {
				text: "",
				approval: {
					id: "ap-1",
					turn_id: "bt_1",
					tool: "computer_screenshot",
					summary:
						"The bot wants to show the people in “Team” this screenshot.",
					scope: "",
					args_hash: "h",
					...approval,
				},
			},
		},
	}) as unknown as IInboxEntry;

const picture = {
	app_id: "ws-1",
	path: "bot/screenshots/a.png",
	bytes: 4,
	sha256: "a".repeat(64),
	mime: "image/png",
};

describe("approvalOf picture", () => {
	test("a card keeps the picture it asks to show, on the card and on its one item", () => {
		expect(approvalOf(card({ picture }))?.picture).toEqual(picture);
		const item = {
			args_hash: "h",
			tool: "computer_screenshot",
			summary: "Show",
			scope: "",
			picture,
		};
		expect(approvalOf(card({ items: [item] }))?.picture).toEqual(picture);
		const batch = approvalOf(
			card({ items: [item, { ...item, args_hash: "h2", picture: undefined }] }),
		);
		expect(batch?.items?.map((entry) => entry.picture)).toEqual([
			picture,
			undefined,
		]);
	});

	test("a picture that is no image of a bot workspace is dropped", () => {
		for (const bad of [
			{ ...picture, mime: "application/pdf" },
			{ ...picture, app_id: "conversation_c1" },
			{ ...picture, path: "" },
			"bot/screenshots/a.png",
		]) {
			expect(approvalOf(card({ picture: bad }))?.picture).toBeUndefined();
		}
	});
});
