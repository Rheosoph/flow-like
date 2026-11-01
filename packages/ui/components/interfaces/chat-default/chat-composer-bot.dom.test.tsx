// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ChatComposerBot } from "./chat-composer-bot";

const reaction = vi.hoisted(() => ({
	use: vi.fn(),
	expression: "curious",
	status: "ready",
}));
vi.mock("../../../hooks/use-bot-expression", () => ({
	useBotExpression: (input: unknown) => {
		reaction.use(input);
		return { expression: reaction.expression, status: reaction.status };
	},
}));
vi.mock("../../bots/bot-avatar", () => ({
	BotAvatar: ({
		expression,
		pose,
		agentState,
	}: { expression: string; pose: string; agentState: string }) => (
		<span
			data-expression={expression}
			data-pose={pose}
			data-agent-state={agentState}
		/>
	),
}));

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	reaction.use.mockClear();
	reaction.status = "ready";
});

test("forwards execution state independently of local emotion model loading and returns to idle", async () => {
	reaction.status = "loading";
	await act(async () => root.render(<ChatComposerBot draft="hello" />));
	expect(
		host.querySelector("[data-agent-state]")?.getAttribute("data-agent-state"),
	).toBe("idle");
	await act(async () =>
		root.render(<ChatComposerBot draft="hello" agentState="working" />),
	);
	expect(
		host.querySelector("[data-agent-state]")?.getAttribute("data-agent-state"),
	).toBe("working");
	expect(
		host.querySelector("[data-expression]")?.getAttribute("data-expression"),
	).toBe("curious");
	reaction.status = "unavailable";
	await act(async () =>
		root.render(<ChatComposerBot draft="hello" agentState="idle" />),
	);
	expect(
		host.querySelector("[data-agent-state]")?.getAttribute("data-agent-state"),
	).toBe("idle");
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

test("passes the draft and recent messages to local inference while retaining idle motion", async () => {
	const context = {
		conversationId: "chat-1",
		messages: [{ role: "assistant" as const, content: "Wie geht es dir?" }],
	};
	await act(async () =>
		root.render(<ChatComposerBot draft="Ich freue mich" context={context} />),
	);
	expect(reaction.use).toHaveBeenLastCalledWith({
		draft: "Ich freue mich",
		messages: context.messages,
		conversationId: "chat-1",
		enabled: true,
	});
	expect(
		host.querySelector("[data-expression]")?.getAttribute("data-expression"),
	).toBe("curious");
	expect(host.querySelector("[data-pose]")?.getAttribute("data-pose")).toBe(
		"idle",
	);
	await act(async () =>
		root.render(
			<ChatComposerBot
				draft=""
				context={{ ...context, conversationId: "chat-2" }}
				composing
			/>,
		),
	);
	expect(reaction.use).toHaveBeenLastCalledWith({
		draft: "",
		messages: context.messages,
		conversationId: "chat-2",
		enabled: false,
	});
});
