import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const actualAvatar = { ...(await import("../identity/identity-avatar")) };
mock.module("../identity/identity-avatar", () => ({
	IdentityAvatar: ({ subject }: { subject: string }) =>
		createElement("span", { "data-avatar": subject }),
}));

const { createRoot } = await import("react-dom/client");
const { ConversationComposer } = await import("./conversation-composer");

const mentionables = [
	{ id: "u1", name: "Ada", kind: "user" as const },
	{ id: "b1", name: "Scout", kind: "bot" as const },
];

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;
const sent: { text: string; mentions: string[] }[] = [];

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../identity/identity-avatar", () => actualAvatar);
	restoreDomGlobals();
});

async function show(withMentions = true) {
	sent.length = 0;
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(
			createElement(ConversationComposer, {
				recipient: "Launch",
				mentionables: withMentions ? mentionables : undefined,
				onSend: (text: string, mentions: string[]) =>
					sent.push({ text, mentions }),
			}),
		),
	);
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

function field() {
	return container.querySelector("textarea") as unknown as HTMLTextAreaElement;
}

async function type(value: string) {
	const input = field();
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			window.HTMLTextAreaElement.prototype,
			"value",
		)?.set?.call(input, value);
		input.setSelectionRange(value.length, value.length);
		input.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}

function suggestions() {
	return [...container.querySelectorAll("button[aria-current]")];
}

async function submit() {
	await act(async () => {
		container
			.querySelector("form")
			?.dispatchEvent(
				new window.Event("submit", { bubbles: true, cancelable: true }),
			);
	});
}

describe("mention picker", () => {
	test("typing @ lists the room's members with avatars", async () => {
		await show();
		await type("hi @");
		expect(suggestions().map((b) => b.textContent)).toEqual(["Ada", "Scout"]);
		expect(container.querySelector('[data-avatar="b1"]')).not.toBeNull();
		await hide();
	});

	test("typing narrows the list", async () => {
		await show();
		await type("hi @sc");
		expect(suggestions().map((b) => b.textContent)).toEqual(["Scout"]);
		await hide();
	});

	test("picking inserts the name and the send carries mentions", async () => {
		await show();
		await type("hi @sc");
		await act(async () => {
			(suggestions()[0] as HTMLElement).dispatchEvent(
				new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }),
			);
		});
		expect(field().value).toBe("hi @Scout ");
		await type("hi @Scout do it");
		await submit();
		expect(sent).toEqual([{ text: "hi @Scout do it", mentions: ["b1"] }]);
		await hide();
	});

	test("a mention whose @name was deleted before sending is dropped", async () => {
		await show();
		await type("@a");
		await act(async () => {
			(suggestions()[0] as HTMLElement).dispatchEvent(
				new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }),
			);
		});
		await type("never mind");
		await submit();
		expect(sent).toEqual([{ text: "never mind", mentions: [] }]);
		await hide();
	});

	test("without mentionables @ is plain text", async () => {
		await show(false);
		await type("hi @");
		expect(suggestions()).toEqual([]);
		await hide();
	});
});
