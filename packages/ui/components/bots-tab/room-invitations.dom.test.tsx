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
		confirmLabel: string;
		group: boolean;
		onConfirm: (limits: unknown) => void;
	}) =>
		props.open
			? createElement(
					"div",
					{ role: "dialog", "data-group": props.group },
					createElement("span", null, `limits for ${props.botName}`),
					createElement(
						"button",
						{ type: "button", onClick: () => props.onConfirm({ mask: 16 }) },
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

function invitation(patch: Partial<IInvitation>): IInvitation {
	return {
		id: "room-1:me",
		conversation_id: "room-1",
		title: "Launch",
		principal_id: "me",
		kind: "user",
		bot_name: null,
		invited_by: "user-a",
		invited_at: null,
		...patch,
	};
}

async function show(item: IInvitation) {
	answers.length = 0;
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(NeedsYouSection, {
				items: [{ type: "invitation", invitation: item }],
				directory,
				text: {} as never,
				onOpenConversation: () => {},
				onOpenAssistant: () => {},
			}),
		),
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

describe("invitations in Needs you", () => {
	test("a person accepts their own invitation directly", async () => {
		const item = invitation({});
		await show(item);
		expect(container.textContent).toContain("Ada invited you to Launch");
		await act(async () => button("Accept").click());
		expect(answers).toEqual([
			{ invitation: item, accept: true, limits: undefined },
		]);
		await hide();
	});

	test("declining sends no limits", async () => {
		const item = invitation({});
		await show(item);
		await act(async () => button("Decline").click());
		expect(answers).toEqual([
			{ invitation: item, accept: false, limits: undefined },
		]);
		await hide();
	});

	test("a bot's owner reviews the limits before the bot joins", async () => {
		const item = invitation({
			id: "room-1:bot-9",
			principal_id: "bot-9",
			kind: "bot",
			bot_name: "Scout",
		});
		await show(item);
		expect(container.textContent).toContain("Ada wants to add Scout to Launch");
		await act(async () => button("Review and accept").click());
		expect(answers).toEqual([]);
		expect(container.textContent).toContain("limits for Scout");
		await act(async () => button("Accept and add").click());
		expect(answers).toEqual([
			{ invitation: item, accept: true, limits: { mask: 16 } },
		]);
		await hide();
	});

	test("declining a bot's invitation needs no dialog", async () => {
		const item = invitation({
			id: "room-1:bot-9",
			principal_id: "bot-9",
			kind: "bot",
			bot_name: "Scout",
		});
		await show(item);
		await act(async () => button("Decline").click());
		expect(answers.map((a) => a.accept)).toEqual([false]);
		expect(container.textContent).not.toContain("limits for Scout");
		await hide();
	});

	test("a direct bot request reviews permissions without group memory controls", async () => {
		await show(
			invitation({
				principal_id: "bot-9",
				kind: "bot",
				bot_name: "Scout",
				conversation_kind: "direct",
			}),
		);
		await act(async () => button("Review and accept").click());
		expect(
			container.querySelector('[role="dialog"]')?.getAttribute("data-group"),
		).toBe("false");
		expect(answers).toEqual([]);
		await hide();
	});
});
