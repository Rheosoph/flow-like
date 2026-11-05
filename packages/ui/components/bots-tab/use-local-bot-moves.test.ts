import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { followMovedBot } from "./use-local-bot-moves";

const realRebind = useGlobalChatStore.getState().rebindBotChat;

afterEach(() => {
	useGlobalChatStore.setState({ rebindBotChat: realRebind });
});

describe("the chat follows a moved bot", () => {
	test("the move waits until the chat moved", async () => {
		let release = () => {};
		const moved: string[] = [];
		useGlobalChatStore.setState({
			rebindBotChat: (from, to) =>
				new Promise<void>((resolve) => {
					release = () => {
						moved.push(`${from}->${to}`);
						resolve();
					};
				}),
		});
		let followed = false;
		const following = Promise.resolve(
			followMovedBot("local_scout", "bot_scout"),
		).then(() => {
			followed = true;
		});
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(followed).toBe(false);
		release();
		await following;
		expect(moved).toEqual(["local_scout->bot_scout"]);
	});

	test("a chat that can't move is reported, not thrown", async () => {
		const warn = spyOn(console, "warn").mockImplementation(() => {});
		useGlobalChatStore.setState({
			rebindBotChat: () => Promise.reject(new Error("Database closed")),
		});
		await expect(
			Promise.resolve(followMovedBot("local_scout", "bot_scout")),
		).resolves.toBeUndefined();
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(warn).toHaveBeenCalledTimes(1);
		warn.mockRestore();
	});
});
