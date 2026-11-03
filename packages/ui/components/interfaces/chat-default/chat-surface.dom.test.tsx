// @vitest-environment happy-dom
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { ISendMessageFunction } from "./chatbox";

const shown = vi.hoisted(() => ({
	welcome: undefined as
		| { title: string; isSending: boolean; send: ISendMessageFunction }
		| undefined,
	transcript: false,
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("./appearance", () => ({
	ChatAppearance: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("./chat-widget-execution", () => ({
	ChatWidgetExecutionProvider: ({ children }: { children: ReactNode }) =>
		children,
}));
vi.mock("./chat", () => ({
	Chat: () => {
		shown.transcript = true;
		return <div data-testid="transcript" />;
	},
}));
vi.mock("./welcome", () => ({
	ChatWelcome: (props: {
		greeting: { title: string };
		isSending: boolean;
		onSendMessage: ISendMessageFunction;
	}) => {
		shown.welcome = {
			title: props.greeting.title,
			isSending: props.isSending,
			send: props.onSendMessage,
		};
		return <div data-testid="welcome" />;
	},
}));

const { ChatSurface } = await import("./chat-surface");

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	shown.welcome = undefined;
	shown.transcript = false;
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

async function render(
	props: Partial<Parameters<typeof ChatSurface>[0]>,
	send: ISendMessageFunction = async () => {},
) {
	await act(async () => {
		root.render(
			<ChatSurface
				chatRef={null}
				runWidgetAction={async () => undefined}
				messages={[]}
				onSendMessage={send}
				{...props}
			/>,
		);
	});
}

describe("the standard chat surface", () => {
	test("reads the transcript before it shows either the welcome or the messages", async () => {
		await render({ loading: true, welcome: { title: "Support" } });
		expect(host.textContent).toContain("Loading conversation...");
		expect(shown.welcome).toBeUndefined();
		expect(shown.transcript).toBe(false);
	});

	test("greets an empty conversation and shows the transcript otherwise", async () => {
		await render({ welcome: { title: "Support" } });
		expect(shown.welcome?.title).toBe("Support");
		expect(shown.transcript).toBe(false);
		await render({});
		expect(shown.transcript).toBe(true);
	});

	test("a send with files from the welcome shows the upload until it settles", async () => {
		let finish: () => void = () => {};
		const send = vi.fn(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		await render({ welcome: { title: "Support" } }, send);
		let sending: Promise<void> | undefined;
		await act(async () => {
			sending = shown.welcome?.send("Here", [new File(["a"], "a.txt")]);
		});
		expect(shown.welcome?.isSending).toBe(true);
		expect(send).toHaveBeenCalledWith(
			"Here",
			[expect.any(File)],
			undefined,
			undefined,
			undefined,
		);
		await act(async () => {
			finish();
			await sending;
		});
		expect(shown.welcome?.isSending).toBe(false);
	});

	test("a text-only send from the welcome shows no upload", async () => {
		let finish: () => void = () => {};
		const send = vi.fn(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		await render({ welcome: { title: "Support" } }, send);
		let sending: Promise<void> | undefined;
		await act(async () => {
			sending = shown.welcome?.send("Hi");
		});
		expect(shown.welcome?.isSending).toBe(false);
		await act(async () => {
			finish();
			await sending;
		});
		expect(send).toHaveBeenCalledOnce();
	});
});
