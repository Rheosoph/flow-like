import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IBotMail,
	IBotMailThread,
} from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = { locales: { ...(await import("@flow-like/locales")) } };
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { MailBubble, ThreadRow, offersEmail } = await import("./bot-mail");
const { botPanelTabs } = await import("./bot-panel-tabs");

const thread: IBotMailThread = {
	id: "t1",
	subject: "Invoice 42",
	participants: ["ann@example.com"],
	last_message_at: new Date().toISOString(),
	last_snippet: "Please find attached",
	message_count: 2,
	unread: 2,
	created_at: new Date().toISOString(),
};

const received: IBotMail = {
	id: "m1",
	direction: "in",
	at: new Date().toISOString(),
	from: [{ name: "Ann", email: "ann@example.com" }],
	to: [{ email: "bot@flow-like.me" }],
	cc: [],
	subject: "Invoice 42",
	text: "Please find attached",
	authenticated: false,
	attachments: [
		{
			name: "invoice.pdf",
			content_type: "application/pdf",
			size: 10,
			handle: {
				app_id: "app1",
				path: "bot/inbox/abc/invoice.pdf",
				bytes: 10,
				sha256: "x",
				mime: "application/pdf",
			},
		},
	],
};

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	restoreDomGlobals();
});

async function show(element: ReturnType<typeof createElement>) {
	container = window.document.createElement("div");
	window.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
	await act(async () => root.render(element));
}
async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

const account = (capabilities: string[], local = false) =>
	({
		id: "bot_a",
		system: null,
		local,
		capabilities,
	}) as never;

describe("the Mail tab", () => {
	test("only an account bot with the email ability offers mail", () => {
		expect(offersEmail(account(["email"]))).toBe(true);
		expect(offersEmail(account(["internet"]))).toBe(false);
		expect(offersEmail(account(["email"], true))).toBe(false);
		expect(botPanelTabs({ botId: "bot_a", bot: account(["email"]) })).toContain(
			"mail",
		);
		expect(
			botPanelTabs({ botId: "bot_a", bot: account(["internet"]) }),
		).not.toContain("mail");
	});

	test("a thread row shows subject, correspondents, snippet and unread count", async () => {
		const open = mock(() => {});
		await show(createElement(ThreadRow, { thread, onOpen: open }));
		expect(container.textContent).toContain("Invoice 42");
		expect(container.textContent).toContain("ann@example.com");
		expect(container.textContent).toContain("Please find attached");
		expect(container.textContent).toContain("2");
		await act(async () => container.querySelector("button")?.click());
		expect(open).toHaveBeenCalledTimes(1);
		await hide();
	});

	test("a received mail flags an unverified sender and links attachments to Files", async () => {
		await show(createElement(MailBubble, { mail: received }));
		expect(container.textContent).toContain("could not be verified");
		const link = container.querySelector("a");
		expect(link?.textContent).toContain("invoice.pdf");
		expect(link?.getAttribute("href")).toContain("/library/config/storage");
		expect(link?.getAttribute("href")).toContain("id=app1");
		await hide();
	});

	test("a sent mail is marked as sent and never as unverified", async () => {
		await show(
			createElement(MailBubble, {
				mail: {
					...received,
					direction: "out",
					authenticated: true,
					from: "bot@flow-like.me",
					attachments: [],
				},
			}),
		);
		expect(container.textContent).toContain("Sent");
		expect(container.textContent).not.toContain("could not be verified");
		await hide();
	});
});
