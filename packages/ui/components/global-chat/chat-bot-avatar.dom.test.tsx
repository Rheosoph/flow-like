// @vitest-environment happy-dom

import { act, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
	type GlobalChatRun,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import {
	ChatBotAvatar,
	GlobalChatBotAvatar,
	useDraftRelay,
} from "./chat-bot-avatar";

const reaction = vi.hoisted(() => ({
	use: vi.fn(),
	expression: "curious",
	status: "ready",
	lingering: false,
}));
vi.mock("../../hooks/use-bot-expression", () => ({
	useBotExpression: (input: unknown) => {
		reaction.use(input);
		return {
			expression: reaction.expression,
			status: reaction.status,
			lingering: reaction.lingering,
		};
	},
}));
vi.mock("../bots/bot-avatar", () => ({
	BotAvatar: ({
		expression,
		pose,
		agentState,
		appearance,
		expressionOverridesState,
	}: {
		expression: string;
		pose: string;
		agentState: string;
		appearance?: { kind?: string };
		expressionOverridesState?: boolean;
	}) => (
		<span
			data-expression={expression}
			data-pose={pose}
			data-agent-state={agentState}
			data-kind={appearance?.kind ?? "default"}
			data-overrides-state={expressionOverridesState}
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
	reaction.lingering = false;
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	useGlobalChatStore.setState({
		runs: {},
		isStreaming: false,
		streamingMessages: [],
		toolPrompt: null,
	});
});

const attribute = (name: string) =>
	host.querySelector(`[${name}]`)?.getAttribute(name);

test("shows the sent reaction while working but preserves attention and error states", async () => {
	reaction.lingering = true;
	for (const agentState of [
		"working",
		"needs-you",
		"error",
		"offline",
	] as const) {
		await act(async () =>
			root.render(<ChatBotAvatar draft="" size={34} agentState={agentState} />),
		);
		expect(attribute("data-overrides-state")).toBe(
			agentState === "working" ? "true" : "false",
		);
	}
});

test("draws the bot it is given with the expression read from the draft and messages", async () => {
	const context = {
		conversationId: "chat-1",
		messages: [{ role: "assistant" as const, content: "Wie geht es dir?" }],
	};
	await act(async () =>
		root.render(
			<ChatBotAvatar
				draft="Ich freue mich"
				context={context}
				agentState="working"
				appearance={{ kind: "flux" }}
				size={34}
			/>,
		),
	);
	expect(reaction.use).toHaveBeenLastCalledWith({
		draft: "Ich freue mich",
		messages: context.messages,
		conversationId: "chat-1",
	});
	expect(attribute("data-kind")).toBe("flux");
	expect(attribute("data-expression")).toBe("curious");
	expect(attribute("data-agent-state")).toBe("working");
	expect(attribute("data-pose")).toBe("idle");
});

function RelayedAvatar({ draft }: Readonly<{ draft?: string }>) {
	const relay = useDraftRelay();
	if (draft !== undefined) queueMicrotask(() => relay.report(draft));
	return (
		<GlobalChatBotAvatar
			relay={relay.listener}
			appearance={{ kind: "pip" }}
			size={34}
		/>
	);
}

test("the header avatar follows the relayed draft and the global chat's state", async () => {
	await act(async () => root.render(<RelayedAvatar />));
	expect(reaction.use).toHaveBeenLastCalledWith(
		expect.objectContaining({ draft: "" }),
	);
	expect(attribute("data-agent-state")).toBe("idle");
	expect(attribute("data-kind")).toBe("pip");

	await act(async () => root.render(<RelayedAvatar draft="Plan my week" />));
	expect(reaction.use).toHaveBeenLastCalledWith(
		expect.objectContaining({ draft: "Plan my week" }),
	);

	await act(async () => useGlobalChatStore.setState({ isStreaming: true }));
	expect(attribute("data-agent-state")).toBe("working");
});

function DraftReporter({
	draft,
	report,
}: { draft: string; report: (draft: string) => void }) {
	useEffect(() => report(draft), [draft, report]);
	return null;
}

function RemountableAvatar({ draft, show }: { draft: string; show: boolean }) {
	const relay = useDraftRelay();
	return (
		<>
			<DraftReporter draft={draft} report={relay.report} />
			{show && <GlobalChatBotAvatar relay={relay.listener} size={34} />}
		</>
	);
}

test("keeps drafts reported before the header subscribes or while it is remounting", async () => {
	await act(async () =>
		root.render(<RemountableAvatar draft="I love this" show />),
	);
	expect(reaction.use).toHaveBeenLastCalledWith(
		expect.objectContaining({ draft: "I love this" }),
	);
	await act(async () =>
		root.render(<RemountableAvatar draft="I'm worried" show={false} />),
	);
	await act(async () =>
		root.render(<RemountableAvatar draft="I'm worried" show />),
	);
	expect(reaction.use).toHaveBeenLastCalledWith(
		expect.objectContaining({ draft: "I'm worried" }),
	);
});

test("needs you only for a prompt of the open chat, or one no run owns", async () => {
	const prompt = (conversationId?: string) => ({
		id: `prompt-${conversationId ?? "unowned"}`,
		kind: "approval" as const,
		conversationId,
		toolName: "call_app_event",
		title: "Approve app event execution",
		respond: () => {},
	});
	useGlobalChatStore.setState({ activeConversationId: "open" });
	await act(async () => root.render(<RelayedAvatar />));
	await act(async () =>
		useGlobalChatStore.setState({ toolPrompt: prompt("other") }),
	);
	expect(attribute("data-agent-state")).toBe("idle");
	await act(async () =>
		useGlobalChatStore.setState({ toolPrompt: prompt("open") }),
	);
	expect(attribute("data-agent-state")).toBe("needs-you");
	await act(async () =>
		useGlobalChatStore.setState({ toolPrompt: prompt(undefined) }),
	);
	expect(attribute("data-agent-state")).toBe("needs-you");
});

test("works only while the open conversation has a live run", async () => {
	const run = (runId: string, conversationId: string) => ({
		runId,
		conversationId,
		selection: {} as GlobalChatRun["selection"],
		label: "Response",
		message: null,
		sourceAttachments: [],
	});
	useGlobalChatStore.setState({ activeConversationId: "open" });
	await act(async () => root.render(<RelayedAvatar />));
	await act(async () =>
		useGlobalChatStore.getState().startRun(run("elsewhere", "other")),
	);
	expect(attribute("data-agent-state")).toBe("idle");
	await act(async () =>
		useGlobalChatStore.getState().startRun(run("here", "open")),
	);
	expect(attribute("data-agent-state")).toBe("working");
	await act(async () => useGlobalChatStore.getState().endRun("here"));
	expect(attribute("data-agent-state")).toBe("idle");
});
