// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { ChatWelcome } from "./welcome";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("./VoiceMode", () => ({ VoiceMode: () => null }));
vi.mock("./ai-disclosure", () => ({ ChatAiDisclosure: () => null }));
vi.mock("./chat-placeholder", () => ({ ChatPlaceholder: () => null }));
vi.mock("./voice-config", () => ({
	resolveChatVoiceConfig: () => ({ mode: "disabled" }),
	isVoiceEnabled: () => false,
}));
vi.mock("./chatbox", async () => {
	const { forwardRef, useImperativeHandle, useRef, useState } = await import(
		"react"
	);
	return {
		ChatBox: forwardRef<
			{ setInput: (value: string) => void; focusInput: () => void },
			{ onContentChange: (value: string) => void }
		>(({ onContentChange }, ref) => {
			const [value, setValue] = useState("");
			const input = useRef<HTMLTextAreaElement>(null);
			useImperativeHandle(ref, () => ({
				setInput: setValue,
				focusInput: () => input.current?.focus(),
			}));
			return (
				<textarea
					ref={input}
					aria-label="Message"
					value={value}
					onInput={(event) => {
						setValue(event.currentTarget.value);
						onContentChange(event.currentTarget.value);
					}}
				/>
			);
		}),
	};
});

const event = { id: "grove", name: "Persona Grove" } as IEvent;
const sendMessage = vi.fn(async () => {});
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	sendMessage.mockClear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

async function render(config?: Partial<IEventPayloadChat>) {
	await act(async () => {
		root.render(
			<ChatWelcome event={event} config={config} onSendMessage={sendMessage} />,
		);
	});
}

function suggestions() {
	return Array.from(
		container.querySelectorAll<HTMLButtonElement>("[data-fl-chat-suggestion]"),
	);
}

function composer() {
	const input = container.querySelector("textarea");
	if (!input) throw new Error("Message composer is missing");
	return input;
}

async function typeMessage(value: string) {
	const input = composer();
	await act(async () => {
		input.value = value;
		input.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

describe("chat welcome suggestions", () => {
	test.each([
		["missing configuration", undefined],
		["missing messages", {}],
		["null messages", { example_messages: null }],
		["empty messages", { example_messages: [] }],
		["blank messages", { example_messages: ["", "  ", "\t\n"] }],
		["non-string messages", { example_messages: [null, 3, { text: "Hi" }] }],
		["non-array messages", { example_messages: "Explore my persona" }],
	] as const)(
		"hides suggestions for %s, including while typing",
		async (_, config) => {
			await render(config as Partial<IEventPayloadChat> | undefined);
			expect(container.querySelector("[data-fl-chat-suggestions]")).toBeNull();
			await typeMessage("Help");
			expect(container.querySelector("[data-fl-chat-suggestions]")).toBeNull();
		},
	);

	test("shows configured messages in order and fills the composer without sending", async () => {
		const messages = [
			"Explore my persona",
			"Reflect on my goals",
			"Review my strengths",
			"Plan my next step",
			"Track my progress",
		];
		await render({
			example_messages: ["", ...messages, "  ", null, { text: "Hi" }] as never,
		});
		expect(suggestions().map((button) => button.textContent)).toEqual(
			messages.slice(0, 4),
		);
		await act(async () => suggestions()[0].click());
		const input = composer();
		expect(input.value).toBe(messages[0]);
		expect(document.activeElement).toBe(input);
		expect(sendMessage).not.toHaveBeenCalled();
	});

	test("filters configured messages and hides the section when none match", async () => {
		await render({
			example_messages: ["Explore my persona", "Reflect on my goals"],
		});
		await typeMessage("GOALS");
		expect(suggestions().map((button) => button.textContent)).toEqual([
			"Reflect on my goals",
		]);
		await typeMessage("zzzzzz");
		expect(container.querySelector("[data-fl-chat-suggestions]")).toBeNull();
	});

	test("removes suggestions when the event configuration is cleared", async () => {
		await render({ example_messages: ["Explore my persona"] });
		expect(suggestions()).toHaveLength(1);
		await render({ example_messages: [] });
		expect(container.querySelector("[data-fl-chat-suggestions]")).toBeNull();
	});
});
