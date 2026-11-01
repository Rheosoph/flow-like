import { describe, expect, test } from "bun:test";
import { IContentType, IRole } from "../../../lib/schema/llm/history";
import { botReactionMessages } from "./bot-reaction-context";
import type { IMessage } from "./chat-db";

function message(
	id: number,
	role: IRole,
	content: IMessage["inner"]["content"],
	sessionId = "chat",
): IMessage {
	return {
		id: `${id}`,
		appId: "app",
		sessionId,
		timestamp: id,
		files: [],
		inner: { role, content },
	};
}

describe("bot reaction conversation context", () => {
	test("keeps the last four visible turns in chronological order without mutating history", () => {
		const history = [6, 2, 5, 1, 4, 3].map((id) =>
			message(id, id % 2 ? IRole.User : IRole.Assistant, `turn ${id}`),
		);
		expect(
			botReactionMessages(history, "chat").map((item) => item.content),
		).toEqual(["turn 3", "turn 4", "turn 5", "turn 6"]);
		expect(history.map((item) => item.timestamp)).toEqual([6, 2, 5, 1, 4, 3]);
	});

	test("excludes other conversations, system/tool messages, reactions and assistant thinking", () => {
		const reaction = message(7, IRole.User, "hidden reaction");
		reaction.interaction = {
			type: "reaction",
			operation: "add",
			emoji: "❤️",
			target: { messageId: "2", role: IRole.Assistant, text: "answer" },
		};
		expect(
			botReactionMessages(
				[
					message(1, IRole.System, "instructions"),
					message(2, IRole.User, "Hallo"),
					message(
						3,
						IRole.Assistant,
						"<think>private reasoning</think>Guten Tag",
					),
					message(4, IRole.ITool, "tool result"),
					message(5, IRole.User, "another conversation", "elsewhere"),
					message(6, IRole.Assistant, "<think>unfinished reasoning"),
					reaction,
				],
				"chat",
			),
		).toEqual([
			{ role: "user", content: "Hallo" },
			{ role: "assistant", content: "Guten Tag" },
		]);
	});

	test("reads text parts only and bounds long context", () => {
		expect(
			botReactionMessages([
				message(1, IRole.User, [
					{ type: IContentType.Text, text: "first " },
					{
						type: IContentType.IImageURL,
						text: "image data",
						image_url: { url: "https://example.test/image" },
					},
					{ type: IContentType.Text, text: "second" },
				]),
			])[0]?.content,
		).toBe("first second");
		expect(
			botReactionMessages([message(2, IRole.User, "a".repeat(2000))])[0]
				?.content.length,
		).toBe(1200);
	});
});
