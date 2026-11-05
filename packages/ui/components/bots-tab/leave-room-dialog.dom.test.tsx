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
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const removals: [string, string][] = [];
let remove: () => Promise<void> = async () => {};
const actualBackend = { ...(await import("../../state/backend-state")) };
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({
		inboxState: {
			removeMember: async (conversationId: string, principalId: string) => {
				removals.push([conversationId, principalId]);
				await remove();
			},
		},
	}),
}));

const { createRoot } = await import("react-dom/client");
const { inboxKeys } = await import("../../hooks/use-inbox");
const { LeaveRoomDialog } = await import("./leave-room-dialog");
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
	mock.module("../../state/backend-state", () => actualBackend);
	restoreDomGlobals();
});

function Harness() {
	const [open, setOpen] = useState(true);
	return createElement(LeaveRoomDialog, {
		open,
		onOpenChange: setOpen,
		conversationId: "room-1",
		principalId: "me",
		onLeave: () => {
			left += 1;
		},
	});
}

async function show(handler: () => Promise<void> = async () => {}) {
	removals.length = 0;
	left = 0;
	remove = handler;
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	client.setQueryData(inboxKeys.conversations(), [{ id: "room-1" }]);
	client.setQueryData(inboxKeys.conversation("room-1"), { id: "room-1" });
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(QueryClientProvider, { client }, createElement(Harness)),
		),
	);
}

function button(label: string) {
	const found = [...window.document.querySelectorAll("button")].find(
		(item) => item.textContent === label,
	);
	if (!found) throw new Error(`Missing button: ${label}`);
	return found;
}

describe("leaving a room", () => {
	test("explains that owned bots leave too and cancellation changes nothing", async () => {
		await show();
		expect(window.document.body.textContent).toContain(
			"You and your bots in this room will leave",
		);
		await act(async () => button("Cancel").click());
		expect(removals).toEqual([]);
		expect(left).toBe(0);
	});

	test("removes the current user, refreshes room queries, and closes the conversation after success", async () => {
		await show();
		await act(async () => button("Leave room").click());
		await settle();
		expect(removals).toEqual([["room-1", "me"]]);
		expect(left).toBe(1);
		expect(client.getQueryState(inboxKeys.conversations())?.isInvalidated).toBe(
			true,
		);
		expect(
			client.getQueryState(inboxKeys.conversation("room-1"))?.isInvalidated,
		).toBe(true);
		expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("keeps the confirmation open with a retry when leaving fails", async () => {
		await show(async () => {
			throw new Error("offline");
		});
		await act(async () => button("Leave room").click());
		await settle();
		expect(left).toBe(0);
		expect(window.document.querySelector('[role="alert"]')).not.toBeNull();
		expect(
			window.document.querySelector('[role="alertdialog"]'),
		).not.toBeNull();
		remove = async () => {};
		await act(async () => button("Leave room").click());
		await settle();
		expect(removals).toHaveLength(2);
		expect(left).toBe(1);
	});

	test("disables both buttons until the removal finishes", async () => {
		let finish = () => {};
		await show(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		await act(async () => button("Leave room").click());
		await settle();
		expect(button("Leave room").disabled).toBe(true);
		expect(button("Cancel").disabled).toBe(true);
		expect(left).toBe(0);
		await act(async () => finish());
		await settle();
		expect(left).toBe(1);
	});
});
