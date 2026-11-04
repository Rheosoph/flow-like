import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type { Root } from "react-dom/client";
import type { MeetingClient, MeetingSession } from "../../lib/meetings/types";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
} from "../bots-tab/testing/dom-harness";

const locales = { ...(await import("@flow-like/locales")) };
const roots: Root[] = [];
beforeAll(() => {
	installDomGlobals();
	mock.module("@flow-like/locales", () => ({
		...locales,
		useTranslation: () => ({ t: translate, i18n: fakeI18n }),
	}));
});
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	await settle();
	document.body.innerHTML = "";
});
afterAll(() => {
	mock.module("@flow-like/locales", () => locales);
	restoreDomGlobals();
});

function fixture() {
	const now = Date.now();
	const meeting = {
		id: "meeting-1",
		conversation_id: "private-room",
		title: "Design review",
		host_id: "host",
		starts_at: new Date(now - 60_000).toISOString(),
		ends_at: new Date(now + 3600_000).toISOString(),
		ended_at: null,
		guests_allowed: true,
	};
	const snapshot: MeetingSession = {
		meeting,
		me: {
			id: "guest-1",
			display_name: "Alex",
			guest: true,
			status: "waiting",
			is_host: false,
		},
		participants: [],
		messages: [],
		server_time: new Date(now).toISOString(),
		room_epoch: 0,
	};
	const client = {
		preview: mock(async () => meeting),
		joinGuest: mock(async () => ({
			participant_id: "guest-1",
			session_token: "session-only",
		})),
		session: mock(async () => snapshot),
		leave: mock(async () => {}),
		create: mock(async () => ({ meeting, invite_token: "invite" })),
	} as unknown as MeetingClient;
	return { client, meeting, snapshot };
}

async function render(element: ReturnType<typeof createElement>) {
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	roots.push(root);
	await act(async () => root.render(element));
	await settle();
	return container;
}

async function click(container: HTMLElement, text: string) {
	const button = [...container.querySelectorAll("button")].find((candidate) =>
		candidate.textContent?.includes(text),
	);
	expect(button).toBeDefined();
	await act(async () => button?.click());
	await settle();
}

describe("meeting join and host controls", () => {
	test("signed-out member links show sign-in without making an account request", async () => {
		const { MeetingPage } = await import("./meeting-page");
		const { client } = fixture();
		const signIn = mock(async () => {});
		const container = await render(
			createElement(MeetingPage, {
				client,
				meetingId: "meeting-1",
				onSignIn: signIn,
			}),
		);
		expect(client.preview).not.toHaveBeenCalled();
		await click(container, "Sign in to join");
		expect(signIn).toHaveBeenCalledTimes(1);
	});
	test("guests wait for admission and leaving does not wait for a network reply", async () => {
		const { MeetingPage } = await import("./meeting-page");
		const { client } = fixture();
		client.leave = mock(() => new Promise<void>(() => {}));
		const container = await render(
			createElement(MeetingPage, {
				client,
				meetingId: "meeting-1",
				inviteToken: "invite",
				displayName: "Alex",
			}),
		);
		await click(container, "Ask to join as a guest");
		expect(client.joinGuest).toHaveBeenCalledWith(
			"meeting-1",
			"invite",
			"Alex",
		);
		expect(container.textContent).toContain("Waiting for the host");
		expect(container.querySelector("video")).toBeNull();
		expect(container.textContent).not.toContain("Meeting chat");
		await click(container, "Leave meeting");
		expect(container.textContent).toContain("Ready to join?");
		expect(container.textContent).not.toContain("Waiting for the host");
	});
	test("host admission and removal target the server participant identity", async () => {
		const { MeetingParticipants } = await import("./meeting-page");
		const { snapshot } = fixture();
		snapshot.me = {
			id: "host-session",
			display_name: "Host",
			status: "admitted",
			guest: false,
			is_host: true,
		};
		snapshot.participants = [
			{
				id: "guest-server-id",
				display_name: "Alex",
				guest: true,
				status: "waiting",
				is_host: false,
			},
		];
		const admit = mock(() => {});
		const remove = mock(() => {});
		const container = await render(
			createElement(MeetingParticipants, {
				snapshot,
				busy: false,
				onAdmit: admit,
				onRemove: remove,
			}),
		);
		await click(container, "Admit");
		await click(container, "Decline");
		expect(admit).toHaveBeenCalledWith("guest-server-id");
		expect(remove).toHaveBeenCalledWith("guest-server-id");
	});
});
