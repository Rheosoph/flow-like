import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IInvitation } from "../../state/backend-state/inbox-state";
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

interface Answer {
	invitation: IInvitation;
	accept: boolean;
	limits?: unknown;
	block?: boolean;
}
const answers: Answer[] = [];

const actualInbox = { ...(await import("../../hooks/use-inbox")) };
mock.module("../../hooks/use-inbox", () => ({
	...actualInbox,
	useAnswerInvitation: () => ({
		isPending: false,
		error: null,
		mutateAsync: async (answer: Answer) => {
			answers.push(answer);
		},
	}),
}));

const actualHoverCard = {
	...(await import("../identity/identity-hover-card")),
};
mock.module("../identity/identity-hover-card", () => ({
	IdentityHoverCard: ({ children }: { children: unknown }) => children,
}));

const actualAvatar = { ...(await import("../identity/identity-avatar")) };
mock.module("../identity/identity-avatar", () => ({
	IdentityAvatar: ({ subject }: { subject: string }) =>
		createElement("span", { "data-avatar": subject }),
}));

const actualDialog = { ...(await import("./room-limits-dialog")) };
mock.module("./room-limits-dialog", () => ({
	RoomLimitsDialog: (props: {
		open: boolean;
		botName: string;
		roomName: string;
		confirmLabel: string;
		onConfirm: (limits: unknown) => void;
	}) =>
		props.open
			? createElement(
					"div",
					{ role: "dialog" },
					createElement("span", null, `limits for ${props.botName}`),
					createElement(
						"button",
						{ type: "button", onClick: () => props.onConfirm({ mask: 4 }) },
						props.confirmLabel,
					),
				)
			: null,
}));

const { createRoot } = await import("react-dom/client");
const { NeedsYouSection } = await import("./needs-you-section");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../hooks/use-inbox", () => actualInbox);
	mock.module("./room-limits-dialog", () => actualDialog);
	mock.module("../identity/identity-avatar", () => actualAvatar);
	mock.module("../identity/identity-hover-card", () => actualHoverCard);
	restoreDomGlobals();
});

const directory = (id: string) =>
	({ name: id === "user-a" ? "Ada" : id, isBot: false }) as never;

function request(patch: Partial<IInvitation> = {}): IInvitation {
	return {
		id: "dm-1:me",
		conversation_id: "dm-1",
		title: null,
		principal_id: "me",
		kind: "user",
		bot_name: null,
		invited_by: "user-a",
		invited_at: null,
		conversation_kind: "direct",
		held_count: 2,
		held: [
			{
				message_id: "m1",
				created_at: "2026-11-03T10:00:00Z",
				envelope: { v: 1, enc: "none", body: { text: "hello there" } },
			},
			{
				message_id: "m2",
				created_at: "2026-11-03T10:01:00Z",
				envelope: { v: 1, enc: "none", body: { text: "are you around?" } },
			},
		],
		...patch,
	};
}

async function mount(node: ReturnType<typeof createElement>) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container as unknown as Element);
	await act(async () => root.render(node));
}

async function show(item: IInvitation) {
	answers.length = 0;
	await mount(
		createElement(NeedsYouSection, {
			items: [{ type: "invitation", invitation: item }],
			directory,
			text: {} as never,
			onOpenConversation: () => {},
			onOpenAssistant: () => {},
		}),
	);
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

function button(label: string) {
	return [...container.querySelectorAll("button")].find((b) =>
		b.textContent?.includes(label),
	) as HTMLElement;
}

describe("message requests in Needs you", () => {
	test("shows who wants to message you and the newest held message", async () => {
		await show(request());
		expect(container.textContent).toContain("Ada wants to message you");
		expect(container.textContent).toContain("Message request");
		expect(container.textContent).toContain("are you around?");
		await hide();
	});

	test("accept answers the request", async () => {
		const item = request();
		await show(item);
		await act(async () => button("Accept").click());
		expect(answers).toEqual([
			{ invitation: item, accept: true, limits: undefined, block: undefined },
		]);
		await hide();
	});

	test("decline and block are different answers", async () => {
		const item = request();
		await show(item);
		await act(async () => button("Decline").click());
		await act(async () => button("Block").click());
		expect(answers.map(({ accept, block }) => ({ accept, block }))).toEqual([
			{ accept: false, block: undefined },
			{ accept: false, block: true },
		]);
		await hide();
	});

	test("a request to someone's bot goes through the owner's limits dialog", async () => {
		const item = request({
			id: "dm-2:bot-9",
			principal_id: "bot-9",
			kind: "bot",
			bot_name: "Scout",
		});
		await show(item);
		expect(container.textContent).toContain("Ada wants to talk to Scout");
		await act(async () => button("Review and accept").click());
		expect(answers).toEqual([]);
		await act(async () => button("Accept and allow").click());
		expect(answers).toEqual([
			{ invitation: item, accept: true, limits: { mask: 4 }, block: undefined },
		]);
		await hide();
	});

	test("a room invitation has no block button", async () => {
		await show(
			request({ conversation_kind: "group", title: "Launch", held: [] }),
		);
		expect(button("Block")).toBeUndefined();
		await hide();
	});
});
