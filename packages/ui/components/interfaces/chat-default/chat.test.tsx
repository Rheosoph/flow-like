import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, forwardRef } from "react";
import { createRoot } from "react-dom/client";

let chatBoxRenderCount = 0;

// `mock.module` is process-global, so this stub also stands in for `next-themes` in every other
// test file of this run. Omitting `ThemeProvider` broke any importer that pulls it in.
mock.module("next-themes", () => ({
	useTheme: () => ({ resolvedTheme: "dark" }),
	ThemeProvider: ({ children }: { children?: React.ReactNode }) => children,
}));

mock.module("react-spinners/PuffLoader", () => ({ default: () => null }));
mock.module("./VoiceMode", () => ({ VoiceMode: () => null }));
mock.module("./interaction", () => ({
	Interaction: () => null,
	InteractionGroup: () => null,
}));
mock.module("./message", () => ({ MessageComponent: () => null }));
mock.module("./use-answer-playback", () => ({
	useAnswerPlayback: () => ({
		analyser: null,
		isPlaying: false,
		stop: () => {},
	}),
}));
mock.module("./voice-config", () => ({
	isVoiceEnabled: () => false,
	resolveChatVoiceConfig: () => ({
		invoke: "manual",
		mode: "record",
		playback: "none",
	}),
}));
mock.module("./chatbox", () => ({
	ChatBox: forwardRef(() => {
		chatBoxRenderCount += 1;
		return null;
	}),
}));

afterAll(() => mock.restore());

describe("Chat active tools", () => {
	test("does not schedule a redundant update for recreated equivalent tool arrays", async () => {
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
		const messages = [];
		const onSendMessage = async () => {};
		const container = window.document.createElement("div");
		window.document.body.append(container);
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
	});
});
