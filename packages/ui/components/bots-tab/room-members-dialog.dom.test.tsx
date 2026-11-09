import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, useState } from "react";
import type { IConversation } from "../../state/backend-state/inbox-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";
import type { CrewDirectory } from "./use-crew-directory";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
const actualBots = { ...(await import("../../hooks/use-bots")) };
mock.module("../../hooks/use-bots", () => ({
	...actualBots,
	useBots: () => ({ data: [{ id: "bot-own" }] }),
}));
const actualBackend = { ...(await import("../../state/backend-state")) };
const removed: [string, string][] = [];
const invited: unknown[] = [];
const created: unknown[] = [];
let fail = false;
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({
		inboxState: {
			invite: async (_id: string, request: unknown) => {
				invited.push(request);
				return { added: ["bot-next"], invited: [], conversation: room() };
			},
			createConversation: async (request: unknown) => {
				created.push(request);
				return room();
			},
			removeMember: async (room: string, member: string) => {
				removed.push([room, member]);
				if (fail) throw new Error("offline");
			},
		},
	}),
}));
const actualAvatar = { ...(await import("../identity/identity-avatar")) };
mock.module("../identity/identity-avatar", () => ({
	IdentityAvatar: () => null,
}));
const actualHover = { ...(await import("../identity/identity-hover-card")) };
mock.module("../identity/identity-hover-card", () => ({
	IdentityHoverCard: ({ children }: { children: unknown }) => children,
}));
const actualPicker = { ...(await import("./principal-picker")) };
mock.module("./principal-picker", () => ({
	...actualPicker,
	PrincipalPicker: ({ onPick }: { onPick: (principal: unknown) => void }) =>
		createElement(
			"div",
			null,
			createElement(
				"button",
				{
					type: "button",
					onClick: () =>
						onPick({
							id: "bot-next",
							kind: "bot",
							label: "Scout",
							mode: "add_now",
						}),
				},
				"Choose Scout",
			),
			createElement(
				"button",
				{
					type: "button",
					onClick: () =>
						onPick({
							id: "bot-foreign",
							kind: "bot",
							label: "PeerBot",
							mode: "invite",
						}),
				},
				"Choose PeerBot",
			),
			createElement(
				"button",
				{
					type: "button",
					onClick: () =>
						onPick({
							id: "user-next",
							kind: "user",
							label: "Ada",
							mode: "invite",
						}),
				},
				"Choose Ada",
			),
		),
}));
const actualLinks = { ...(await import("./room-invite-links")) };
mock.module("./room-invite-links", () => ({ RoomInviteLinks: () => null }));
const actualBlock = { ...(await import("./principal-block-action")) };
mock.module("./principal-block-action", () => ({
	...actualBlock,
	PrincipalBlockButton: ({ principal }: { principal: { id: string } }) =>
		createElement(
			"button",
			{ type: "button", "data-block": principal.id },
			"Block",
		),
}));

const { createRoot } = await import("react-dom/client");
const { RoomMembersDialog } = await import("./room-members-dialog");
const { NewRoomDialog } = await import("./new-room-dialog");
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let left = 0;

beforeAll(installDomGlobals);
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	client.clear();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../hooks/use-bots", () => actualBots);
	mock.module("../../state/backend-state", () => actualBackend);
	mock.module("../identity/identity-avatar", () => actualAvatar);
	mock.module("../identity/identity-hover-card", () => actualHover);
	mock.module("./principal-picker", () => actualPicker);
	mock.module("./room-invite-links", () => actualLinks);
	mock.module("./principal-block-action", () => actualBlock);
	restoreDomGlobals();
});

const directory: CrewDirectory = (id) => ({
	id,
	name: id,
	isMe: id === "me",
	isBot: id.startsWith("bot-"),
	ownerId: id === "bot-own" ? "me" : id === "bot-other" ? "them" : undefined,
	isPending: false,
	isResolved: true,
});
function room(createdBy = "them"): IConversation {
	return {
		id: "room-1",
		kind: "group",
		title: "Launch",
		encryption: "plain",
		created_by: createdBy,
		created_at: "2026-11-05T12:00:00Z",
		last_activity_at: "2026-11-05T12:00:00Z",
		unread_count: 0,
		last_entry: null,
		members: ["me", "them", "bot-own", "bot-other"].map((id) => ({
			principal_id: id,
			kind: id.startsWith("bot-") ? "bot" : "user",
			added_by: "me",
			joined_at: "2026-11-05T12:00:00Z",
		})),
	};
}
function Harness({
	conversation,
	me,
}: { conversation: IConversation; me: string }) {
	const [open, setOpen] = useState(true);
	return createElement(RoomMembersDialog, {
		open,
		onOpenChange: setOpen,
		conversation,
		directory,
		me,
		source: {} as never,
		onLeave: () => {
			left += 1;
		},
	});
}
async function show(conversation = room(), me = "me", newRoom = false) {
	removed.length = 0;
	invited.length = 0;
	created.length = 0;
	fail = false;
	left = 0;
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				newRoom
					? createElement(NewRoomDialog, {
							open: true,
							onOpenChange: () => {},
							source: {} as never,
							onCreated: () => {},
						})
					: createElement(Harness, { conversation, me }),
			),
		),
	);
}
function removalLabels() {
	return [...window.document.querySelectorAll("button[aria-label]")]
		.map((button) => button.getAttribute("aria-label"))
		.filter((label) => label?.startsWith("Remove "));
}
function button(label: string) {
	const found = [...window.document.querySelectorAll("button")].find(
		(item) =>
			item.textContent === label || item.getAttribute("aria-label") === label,
	);
	if (!found) throw new Error(`Missing button: ${label}`);
	return found;
}

describe("room member controls", () => {
	test("members can remove their own bots and block other people or their bots", async () => {
		await show();
		expect(removalLabels()).toEqual(["Remove bot-own from room"]);
		expect(
			[...window.document.querySelectorAll("[data-block]")].map((button) =>
				button.getAttribute("data-block"),
			),
		).toEqual(["them", "bot-other"]);
		await act(async () => button("Remove bot-own from room").click());
		expect(removed).toEqual([]);
		expect(window.document.body.textContent).toContain(
			"This bot will leave the room",
		);
		await act(async () => button("Remove from room").click());
		await settle();
		expect(removed).toEqual([["room-1", "bot-own"]]);
		expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("the creator can remove people and foreign bots, but sees no self-removal action", async () => {
		await show(room("me"));
		expect(removalLabels()).toEqual([
			"Remove them from room",
			"Remove bot-own from room",
			"Remove bot-other from room",
		]);
		await act(async () => button("Remove them from room").click());
		expect(window.document.body.textContent).toContain(
			"This person and their bots in the room will leave",
		);
		fail = true;
		await act(async () => button("Remove from room").click());
		await settle();
		expect(window.document.querySelector('[role="alert"]')).not.toBeNull();
		expect(
			window.document.querySelector('[role="alertdialog"]'),
		).not.toBeNull();
	});

	test("someone outside the room gets no removal or leave action", async () => {
		await show(room(), "outsider");
		expect(removalLabels()).toEqual([]);
		expect(window.document.body.textContent).not.toContain("Leave room");
	});

	test("leaving from the members dialog closes both the dialog and the active conversation", async () => {
		await show();
		await act(async () => button("Leave room").click());
		const dialog = window.document.querySelector('[role="alertdialog"]');
		const confirm = [...(dialog?.querySelectorAll("button") ?? [])].find(
			(item) => item.textContent === "Leave room",
		);
		expect(confirm).toBeDefined();
		await act(async () => confirm?.click());
		await settle();
		expect(removed).toEqual([["room-1", "me"]]);
		expect(left).toBe(1);
		expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	});
});

describe("bot setup before joining", () => {
	test("adding an owned bot reviews independent memory choices before any invite request", async () => {
		await show();
		await act(async () => button("Choose Scout").click());
		await act(async () => button("Continue to bot setup").click());
		expect(invited).toEqual([]);
		expect(
			window.document.querySelector('[aria-label="Memory settings for Scout"]'),
		).not.toBeNull();
		const owner = window.document.querySelector(
			'button[role="switch"][aria-label="Use 1:1 memory"]',
		);
		await act(async () => (owner as unknown as HTMLElement).click());
		await act(async () => button("Add or invite 1").click());
		await settle();
		expect(invited).toEqual([
			{
				principal_ids: ["bot-next"],
				bot_limits: {
					"bot-next": {
						use_owner_memory: true,
						use_shared_memory: false,
						share_memory: false,
					},
				},
			},
		]);
	});
	test("another person's bot goes through its owner's invitation review", async () => {
		await show();
		await act(async () => button("Choose PeerBot").click());
		expect(window.document.body.textContent).not.toContain(
			"Continue to bot setup",
		);
		await act(async () => button("Add or invite 1").click());
		await settle();
		expect(invited).toEqual([{ principal_ids: ["bot-foreign"] }]);
	});
	test("new groups keep setup choices when going back and send them with creation", async () => {
		await show(room(), "me", true);
		await act(async () => button("Choose Scout").click());
		await act(async () => button("Continue to bot setup").click());
		expect(created).toEqual([]);
		const sharing = () =>
			window.document.querySelector(
				'button[role="switch"][aria-label="Contribute to shared memory"]',
			) as unknown as HTMLElement;
		await act(async () => sharing().click());
		await act(async () => button("Back").click());
		expect(created).toEqual([]);
		await act(async () => button("Continue to bot setup").click());
		expect(sharing().getAttribute("aria-checked")).toBe("true");
		await act(async () => button("Create room").click());
		await settle();
		expect(created).toEqual([
			{
				kind: "group",
				title: null,
				members: ["bot-next"],
				bot_limits: {
					"bot-next": {
						use_owner_memory: false,
						use_shared_memory: false,
						share_memory: true,
					},
				},
			},
		]);
	});
});
