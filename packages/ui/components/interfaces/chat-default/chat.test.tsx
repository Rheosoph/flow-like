import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, forwardRef } from "react";
import { createRoot } from "react-dom/client";
import type { IChatRef } from "./chat";
import type { IMessage } from "./chat-db";

let chatBoxRenderCount = 0;
const messageRenders: Array<{
	id: string;
	loading?: boolean;
	anchoredContent?: unknown;
}> = [];

/** The test body pays the first import of `./chat`, which pulls in the whole `lib` barrel. */
const TIMEOUT_MS = 20_000;

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll. Radix picks its layout effect when first imported,
// so the real modules load under a document.
const globalDescriptors = [
	"document",
	"HTMLElement",
	"Node",
	"navigator",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"window",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	nextThemes: { ...(await import("next-themes")) },
	puffLoader: { ...(await import("react-spinners/PuffLoader")) },
	voiceMode: { ...(await import("./VoiceMode")) },
	interaction: { ...(await import("./interaction")) },
	message: { ...(await import("./message")) },
	answerPlayback: { ...(await import("./use-answer-playback")) },
	voiceConfig: { ...(await import("./voice-config")) },
	chatbox: { ...(await import("./chatbox")) },
};

mock.module("next-themes", () => ({
	...actual.nextThemes,
	useTheme: () => ({ resolvedTheme: "dark" }),
}));

mock.module("react-spinners/PuffLoader", () => ({
	...actual.puffLoader,
	default: () => null,
}));
mock.module("./VoiceMode", () => ({
	...actual.voiceMode,
	VoiceMode: () => null,
}));
mock.module("./interaction", () => ({
	...actual.interaction,
	Interaction: () => null,
	InteractionGroup: () => null,
}));
mock.module("./message", () => ({
	...actual.message,
	MessageComponent: (props: {
		message: IMessage;
		loading?: boolean;
		anchoredContent?: unknown;
	}) => {
		messageRenders.push({
			id: props.message.id,
			loading: props.loading,
			anchoredContent: props.anchoredContent,
		});
		return null;
	},
}));
mock.module("./use-answer-playback", () => ({
	...actual.answerPlayback,
	useAnswerPlayback: () => ({
		analyser: null,
		isPlaying: false,
		stop: () => {},
	}),
}));
mock.module("./voice-config", () => ({
	...actual.voiceConfig,
	isVoiceEnabled: () => false,
	resolveChatVoiceConfig: () => ({
		invoke: "manual",
		mode: "record",
		playback: "none",
	}),
}));
mock.module("./chatbox", () => ({
	...actual.chatbox,
	ChatBox: forwardRef(() => {
		chatBoxRenderCount += 1;
		return null;
	}),
}));

afterAll(() => {
	mock.restore();
	mock.module("next-themes", () => actual.nextThemes);
	mock.module("react-spinners/PuffLoader", () => actual.puffLoader);
	mock.module("./VoiceMode", () => actual.voiceMode);
	mock.module("./interaction", () => actual.interaction);
	mock.module("./message", () => actual.message);
	mock.module("./use-answer-playback", () => actual.answerPlayback);
	mock.module("./voice-config", () => actual.voiceConfig);
	mock.module("./chatbox", () => actual.chatbox);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

describe("Chat active tools", () => {
	test(
		"does not schedule a redundant update for recreated equivalent tool arrays",
		async () => {
			const window = new Window();
			Object.assign(globalThis, {
				document: window.document,
				HTMLElement: window.HTMLElement,
				Node: window.Node,
				navigator: window.navigator,
				requestAnimationFrame: window.requestAnimationFrame.bind(window),
				cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
				window,
			});
			Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

			const { Chat } = await import("./chat");
			const messages: IMessage[] = [];
			const onSendMessage = async () => {};
			const container = document.createElement("div");
			document.body.append(container);
			const root = createRoot(container);

			chatBoxRenderCount = 0;
			await act(async () => {
				root.render(
					<Chat
						messages={messages}
						onSendMessage={onSendMessage}
						config={{ default_tools: [], tools: [] }}
					/>,
				);
			});

			const renderCountAfterMount = chatBoxRenderCount;
			await act(async () => {
				root.render(
					<Chat
						messages={messages}
						onSendMessage={onSendMessage}
						config={{ default_tools: [], tools: [] }}
					/>,
				);
			});

			expect(chatBoxRenderCount - renderCountAfterMount).toBe(1);

			await act(async () => root.unmount());
			window.close();
		},
		TIMEOUT_MS,
	);
});

describe("Chat message inline content", () => {
	test(
		"hands each settled and live message its own anchored content",
		async () => {
			const window = new Window();
			Object.assign(globalThis, {
				document: window.document,
				HTMLElement: window.HTMLElement,
				Node: window.Node,
				navigator: window.navigator,
				requestAnimationFrame: window.requestAnimationFrame.bind(window),
				cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
				window,
			});
			Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

			const { Chat } = await import("./chat");
			const message = (id: string, timestamp: number): IMessage =>
				({
					id,
					appId: "app",
					sessionId: "s1",
					files: [],
					timestamp,
					inner: { role: "assistant", content: "text" },
				}) as IMessage;
			const settledContent = [{ key: "page", offset: 2, node: null }];
			const liveContent = [{ key: "chat", offset: 0, node: null }];
			const messageInlineContent = new Map([
				["settled", settledContent],
				["live", liveContent],
			]);
			const ref = { current: null as IChatRef | null };
			const container = document.createElement("div");
			document.body.append(container);
			const root = createRoot(container);

			messageRenders.length = 0;
			await act(async () => {
				root.render(
					<Chat
						ref={ref}
						sessionId="s1"
						messages={[message("settled", 1), message("other", 2)]}
						onSendMessage={async () => {}}
						messageInlineContent={messageInlineContent}
					/>,
				);
			});
			await act(async () => {
				ref.current?.pushCurrentMessageUpdate(message("live", 3));
				await new Promise((resolve) => setTimeout(resolve, 50));
			});

			const last = (id: string) =>
				messageRenders.filter((render) => render.id === id).at(-1);
			expect(last("settled")?.anchoredContent).toBe(settledContent);
			expect(last("other")?.anchoredContent).toBeUndefined();
			expect(last("live")).toMatchObject({ loading: true });
			expect(last("live")?.anchoredContent).toBe(liveContent);

			await act(async () => root.unmount());
			window.close();
		},
		TIMEOUT_MS,
	);
});
