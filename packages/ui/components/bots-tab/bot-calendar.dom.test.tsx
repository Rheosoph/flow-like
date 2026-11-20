import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { act } from "react";
import type {
	BotCapability,
	IBotTask,
	ICalendarRange,
	ICreateBotTaskRequest,
	IUpdateBotTaskRequest,
} from "../../state/backend-state/bot-state";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
// Radix picks its layout effect when it loads, so a document must exist before the pane is imported.
import { restoreDomGlobals } from "./testing/dom-harness";
import {
	backendWith,
	botOf,
	mountWithBackend,
} from "./testing/mount-with-backend";

const actualOidc = { ...(await import("react-oidc-context")) };
mock.module("react-oidc-context", () => ({
	...actualOidc,
	useAuth: () => ({
		isAuthenticated: true,
		user: { access_token: "token", profile: { sub: "me" } },
	}),
}));

function tomorrowAt(hours: number): string {
	const day = new Date();
	day.setDate(day.getDate() + 1);
	day.setHours(hours, 0, 0, 0);
	return day.toISOString();
}

const own: IBotTask = {
	id: "task_dentist",
	bot_id: "bot_sec",
	kind: "event",
	title: "Dentist",
	instruction: "Bring the card",
	enabled: true,
	created_by: "owner",
	created_at: "2026-11-01T00:00:00Z",
	event: {
		starts_at: tomorrowAt(9),
		ends_at: tomorrowAt(10),
		wake_offsets: [15],
	},
};

const report: IBotTask = {
	id: "task_report",
	bot_id: "bot_sec",
	kind: "once",
	title: "Weekly report",
	instruction: "Write it",
	at: tomorrowAt(14),
	enabled: true,
	created_by: "owner",
	created_at: "2026-11-01T00:00:00Z",
};

const range: ICalendarRange = {
	from: "",
	to: "",
	entries: [
		{
			id: "task_dentist",
			source: "bot",
			title: "Dentist",
			starts_at: tomorrowAt(9),
			ends_at: tomorrowAt(10),
			task_id: "task_dentist",
			wake: { offsets_minutes: [15], own: false },
		},
		{
			id: "ext.google.c1.7072696d617279.6576",
			source: "external",
			title: "Standup",
			starts_at: tomorrowAt(11),
			ends_at: tomorrowAt(12),
			calendar: "Work",
		},
	],
	sources: [
		{
			connector_id: "c1",
			label: "Work",
			provider: "google",
			error: "403 Forbidden",
		},
	],
};

const calls = {
	listed: [] as { from: string; to: string }[],
	updated: [] as { taskId: string; request: IUpdateBotTaskRequest }[],
	created: [] as ICreateBotTaskRequest[],
};

const botState = {
	listCalendar: async (_botId: string, from: string, to: string) => {
		calls.listed.push({ from, to });
		return range;
	},
	listTasks: async () => [own, report],
	getCalendarWake: async () => ({
		enabled: true,
		offsets_minutes: [15],
		internal: true,
		external: true,
		only_matching: [],
		skip_matching: [],
		skip_all_day: true,
		quiet: true,
	}),
	createTask: async (_botId: string, request: ICreateBotTaskRequest) => {
		calls.created.push(request);
		return { ...own, id: "task_new" };
	},
	updateTask: async (
		_botId: string,
		taskId: string,
		request: IUpdateBotTaskRequest,
	) => {
		calls.updated.push({ taskId, request });
		return own;
	},
	deleteTask: async () => {},
};

function dayStart(offset: number): string {
	const day = new Date();
	day.setDate(day.getDate() + offset);
	day.setHours(0, 0, 0, 0);
	return day.toISOString();
}

function fact(id: string, text: string, patch: Partial<BotFact>): BotFact {
	return {
		id,
		text,
		kind: "event",
		subject: "user",
		source: "user_stated",
		confidence: 1,
		recorded_at: "2026-11-01T00:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [],
		...patch,
	};
}

const facts: BotFact[] = [
	fact("f_birthday", "Anna's birthday", { time: dayStart(2) }),
	fact("f_flight", "Flight to Lisbon departs", { time: tomorrowAt(18) }),
	fact("f_holiday", "On holiday in Lisbon", {
		valid_from: dayStart(4),
		valid_to: dayStart(6),
	}),
	fact("f_parcel", "Parcel pickup code 4711", {
		time: tomorrowAt(12),
		expires_at: dayStart(-1),
	}),
];

const factsUrls: string[] = [];
const realFetch = globalThis.fetch;

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
	calls.listed.length = 0;
	calls.updated.length = 0;
	calls.created.length = 0;
	factsUrls.length = 0;
	globalThis.fetch = realFetch;
});
afterAll(() => {
	mock.module("react-oidc-context", () => actualOidc);
	restoreDomGlobals();
});

async function settle() {
	for (let attempt = 0; attempt < 6; attempt++)
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

async function click(element: Element | null | undefined) {
	if (!element) throw new Error("nothing to click");
	await act(async () => {
		element.dispatchEvent(
			new globalThis.window.MouseEvent("click", {
				bubbles: true,
				cancelable: true,
			}) as never,
		);
	});
	await settle();
}

function byText(root: ParentNode, selector: string, text: string) {
	return [...root.querySelectorAll(selector)].find(
		(element) => element.textContent?.trim() === text,
	);
}

const { BotCalendarPane } = await import("./bot-calendar");

async function mountPane(
	capabilities: BotCapability[] = ["scheduling", "calendar"],
	onTab?: (tab: string) => void,
) {
	const mounted = await mountWithBackend(
		<BotCalendarPane
			bot={botOf("bot_sec", { capabilities, location: "server" })}
			name="Secretary"
			onTab={onTab}
		/>,
		backendWith([], {
			...botState,
			getWorkspace: async () => ({ app_id: "ws_sec" }),
		}),
	);
	unmount = mounted.unmount;
	await settle();
	return mounted.container;
}

/** The web reads facts from the hub; the desktop would ask Tauri. */
function serveFacts() {
	globalThis.fetch = (async (input: RequestInfo | URL) => {
		const url = String(input);
		if (!url.endsWith("/bots/bot_sec/memory/facts")) {
			throw new Error(`unexpected request ${url}`);
		}
		factsUrls.push(url);
		return new Response(JSON.stringify({ facts }), { status: 200 });
	}) as typeof fetch;
}

describe("the Calendar tab", () => {
	test("lays the bot's events, connected calendars and tasks over each other", async () => {
		const container = await mountPane();
		const text = container.textContent ?? "";
		expect(text).toContain("Dentist");
		expect(text).toContain("Standup");
		expect(text).toContain("Weekly report");
		expect(text).toContain("Couldn't read Work: 403 Forbidden");
		expect(calls.listed).toHaveLength(1);
		const { from, to } = calls.listed[0];
		expect(Date.parse(to) - Date.parse(from)).toBe(14 * 86_400_000);

		await click(container.querySelector('button[data-layer="tasks"]'));
		expect(container.textContent).not.toContain("Weekly report");
		expect(container.textContent).toContain("Dentist");
	});

	test("an own event opens its details, where its wake-up is switched off", async () => {
		const container = await mountPane();
		await click(byText(container, "span", "Dentist")?.parentElement);
		const body = container.ownerDocument.body;
		expect(body.textContent).toContain("Secretary's calendar");
		expect(body.textContent).toContain("Wakes Secretary 15 min before");
		await click(byText(body, "button", "Never"));
		expect(calls.updated).toEqual([
			{
				taskId: "task_dentist",
				request: {
					event: {
						starts_at: own.event?.starts_at as string,
						ends_at: own.event?.ends_at,
						wake: { enabled: false, offsets_minutes: [] },
					},
				},
			},
		]);
	});

	test("a connected event's wake-up creates its wake-up row", async () => {
		const container = await mountPane();
		await click(byText(container, "span", "Standup")?.parentElement);
		const body = container.ownerDocument.body;
		expect(body.textContent).toContain("Work");
		await click(byText(body, "button", "Wake"));
		expect(calls.created).toHaveLength(1);
		expect(calls.created[0]).toMatchObject({
			kind: "event",
			title: "Standup",
			event: {
				external: {
					connector_id: "c1",
					provider: "google",
					calendar_id: "primary",
					event_id: "ev",
				},
				wake: { enabled: true, offsets_minutes: [] },
			},
		});
	});

	test("the Memories layer puts remembered days, moments and spans in the calendar, never expired ones", async () => {
		serveFacts();
		const container = await mountPane(["scheduling", "calendar", "memory"]);
		expect(factsUrls).toHaveLength(1);
		expect(
			container.querySelector('button[data-layer="memories"]')?.textContent,
		).toBe("Memories");
		const text = container.textContent ?? "";
		expect(text).toContain("Anna's birthday");
		expect(text).toContain("Flight to Lisbon departs");
		expect(text).toContain("On holiday in Lisbon");
		expect(text).not.toContain("Parcel pickup code 4711");
		expect(text).toContain("Dentist");

		await click(container.querySelector('button[data-layer="memories"]'));
		expect(container.textContent).not.toContain("Anna's birthday");
		expect(container.textContent).not.toContain("On holiday in Lisbon");
		expect(container.textContent).toContain("Dentist");

		await click(container.querySelector('button[data-layer="memories"]'));
		expect(container.textContent).toContain("Anna's birthday");
	});

	test("a memory opens its memory card, and the Memory tab is one click away", async () => {
		serveFacts();
		const opened: string[] = [];
		const container = await mountPane(["memory"], (tab) => opened.push(tab));
		expect(container.querySelector("button[data-layer]")).toBeNull();
		await click(byText(container, "span", "Anna's birthday")?.parentElement);
		const dialog = container.ownerDocument.querySelector('[role="dialog"]');
		expect(dialog?.textContent).toContain("Secretary remembers");
		expect(dialog?.querySelector("[data-memory-id]")?.textContent).toContain(
			"Anna's birthday",
		);
		await click(byText(dialog as ParentNode, "button", "Show in Memory"));
		expect(opened).toEqual(["memory"]);
	});
});
