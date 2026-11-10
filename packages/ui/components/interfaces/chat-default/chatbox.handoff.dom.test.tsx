// @vitest-environment happy-dom
import { act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ChatBox, type ISendMessageFunction } from "./chatbox";
import { ComposerHandoffProvider } from "./chatbox/composer-handoff";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

/** `stage` stands for welcome → transcript: a new key mounts a fresh composer in its place. */
function Surface({
	stage,
	scope = "conversation",
	onSend = async () => {},
}: Readonly<{
	stage: string;
	scope?: string;
	onSend?: ISendMessageFunction;
}>) {
	return (
		<ComposerHandoffProvider scope={scope}>
			<div key={stage} data-stage={stage}>
				<ChatBox
					onSendMessage={onSend}
					fileUpload
					audioInput={false}
					availableTools={[]}
					defaultActiveTools={[]}
				/>
			</div>
		</ComposerHandoffProvider>
	);
}

const field = () => host.querySelector("textarea") as HTMLTextAreaElement;

async function type(value: string) {
	const setValue = Object.getOwnPropertyDescriptor(
		HTMLTextAreaElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setValue?.call(field(), value);
		field().dispatchEvent(new Event("input", { bubbles: true }));
	});
}

describe("composer hand-off", () => {
	test("a composer swapped mid-draft keeps the text and the focus", async () => {
		await act(async () => root.render(<Surface stage="welcome" />));
		await type("half a thought");
		field().focus();

		await act(async () => root.render(<Surface stage="transcript" />));

		expect(host.querySelector('[data-stage="welcome"]')).toBeNull();
		expect(field().value).toBe("half a thought");
		expect(document.activeElement).toBe(field());
	});

	test("a send that swaps the composer hands over the focus, not the sent text", async () => {
		const sent = vi.fn();
		function SwapOnSend() {
			const [stage, setStage] = useState("welcome");
			return (
				<Surface
					stage={stage}
					onSend={async (content) => {
						sent(content);
						setStage("transcript");
					}}
				/>
			);
		}
		await act(async () => root.render(<SwapOnSend />));
		await type("hello");
		field().focus();

		await act(async () => {
			host
				.querySelector("form")
				?.dispatchEvent(
					new Event("submit", { bubbles: true, cancelable: true }),
				);
		});

		expect(sent).toHaveBeenCalledWith("hello");
		expect(host.querySelector('[data-stage="transcript"]')).not.toBeNull();
		expect(field().value).toBe("");
		expect(document.activeElement).toBe(field());
	});

	test("a draft never follows the user into another conversation", async () => {
		await act(async () => root.render(<Surface stage="a" scope="first" />));
		await type("for the first chat");
		field().focus();

		await act(async () => root.render(<Surface stage="b" scope="second" />));

		expect(field().value).toBe("");
		expect(document.activeElement).not.toBe(field());
	});

	test("pressing Send leaves the focus in the field", async () => {
		await act(async () => root.render(<Surface stage="chat" />));
		await type("hi");
		const send = host.querySelector<HTMLButtonElement>('button[type="submit"]');
		const press = new MouseEvent("mousedown", {
			bubbles: true,
			cancelable: true,
		});
		send?.dispatchEvent(press);
		expect(press.defaultPrevented).toBe(true);
	});
});
