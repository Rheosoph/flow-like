import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type {
	BotCapability,
	ICalendarFeed,
	IUpdateCalendarFeed,
} from "../../state/backend-state/bot-state";
// Radix picks its layout effect when it loads, so a document must exist before the dialog is imported.
import { restoreDomGlobals } from "./testing/dom-harness";
import {
	backendWith,
	botOf,
	mountWithBackend,
} from "./testing/mount-with-backend";

const URL_OF = "https://api.example.com/api/v1/calendar-feeds/tok.ics";

function feedOf(overrides: Partial<ICalendarFeed> = {}): ICalendarFeed {
	return {
		url: URL_OF,
		webcal_url: "webcal://api.example.com/api/v1/calendar-feeds/tok.ics",
		include_events: true,
		include_tasks: true,
		include_memories: false,
		include_notes: false,
		created_at: "2026-11-20T10:00:00Z",
		last_fetched_at: null,
		...overrides,
	};
}

const state = {
	feed: null as ICalendarFeed | null,
	reads: 0,
	updates: [] as IUpdateCalendarFeed[],
	rotated: 0,
	deleted: 0,
};

const botState = {
	getCalendarFeed: async () => {
		state.reads++;
		return state.feed;
	},
	updateCalendarFeed: async (_botId: string, update: IUpdateCalendarFeed) => {
		state.updates.push(update);
		state.feed = { ...(state.feed ?? feedOf()), ...update };
		return state.feed;
	},
	rotateCalendarFeed: async () => {
		state.rotated++;
		state.feed = feedOf({ url: `${URL_OF}?new` });
		return state.feed;
	},
	deleteCalendarFeed: async () => {
		state.deleted++;
		state.feed = null;
	},
};

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
	state.feed = null;
	state.reads = 0;
	state.updates.length = 0;
	state.rotated = 0;
	state.deleted = 0;
});
afterAll(() => restoreDomGlobals());

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

const { CalendarFeedButton } = await import("./bot-calendar-feed");

async function mountButton(
	capabilities: BotCapability[],
	options: { local?: boolean } = {},
) {
	const mounted = await mountWithBackend(
		<CalendarFeedButton
			bot={botOf("bot_sec", { capabilities, local: options.local })}
			name="Secretary"
		/>,
		backendWith([], botState),
	);
	unmount = mounted.unmount;
	return {
		container: mounted.container,
		body: mounted.container.ownerDocument.body,
	};
}

const OPEN = "Show in your calendar";

describe("the calendar link button", () => {
	test("a bot with no calendar, scheduling or memory has none", async () => {
		const { container } = await mountButton(["internet"]);
		expect(byText(container, "button", OPEN)).toBeUndefined();
	});

	test("without a link it explains itself and creates one with the chosen layers", async () => {
		const { container, body } = await mountButton([
			"calendar",
			"scheduling",
			"memory",
		]);
		await click(byText(container, "button", OPEN));
		expect(body.textContent).toContain("It is read only");
		expect(body.textContent).not.toContain("anyone who has it");

		const switches = [...body.querySelectorAll('button[role="switch"]')];
		expect(switches).toHaveLength(4);
		await click(switches[2]);
		expect(body.textContent).toContain("anyone who has it");

		await click(byText(body, "button", "Create link"));
		expect(state.updates).toEqual([
			{
				include_events: true,
				include_tasks: true,
				include_memories: true,
				include_notes: false,
			},
		]);
		expect(body.textContent).toContain("Apple Calendar");
	});

	test("an existing link offers every calendar app with its subscribe address", async () => {
		state.feed = feedOf();
		const { container, body } = await mountButton(["calendar"]);
		await click(byText(container, "button", OPEN));
		for (const label of [
			"Apple Calendar",
			"Google Calendar",
			"Outlook.com",
			"Microsoft 365",
			"Copy link",
			"New link",
			"Turn off",
		]) {
			expect(byText(body, "button", label)).toBeDefined();
		}
		expect(body.textContent).toContain("only every few hours");
		const input = body.querySelector("input") as HTMLInputElement;
		expect(input.value).toBe(URL_OF);
		expect(body.querySelectorAll('button[role="switch"]')).toHaveLength(2);
	});

	test("a switch changes one layer and keeps the link", async () => {
		state.feed = feedOf();
		const { container, body } = await mountButton(["calendar"]);
		await click(byText(container, "button", OPEN));
		await click(body.querySelector('button[role="switch"]'));
		expect(state.updates).toEqual([{ include_events: false }]);
	});

	test("a new link and turning off ask first", async () => {
		state.feed = feedOf();
		const { container, body } = await mountButton(["calendar"]);
		await click(byText(container, "button", OPEN));

		await click(byText(body, "button", "New link"));
		expect(state.rotated).toBe(0);
		expect(body.textContent).toContain("stop updating");
		await click(byText(body, "button", "Cancel"));
		expect(byText(body, "button", "Apple Calendar")).toBeDefined();

		await click(byText(body, "button", "New link"));
		await click(byText(body, "button", "Replace link"));
		expect(state.rotated).toBe(1);
		expect((body.querySelector("input") as HTMLInputElement).value).toBe(
			`${URL_OF}?new`,
		);

		await click(byText(body, "button", "Turn off"));
		expect(state.deleted).toBe(0);
		await click(byText(body, "button", "Turn off"));
		expect(state.deleted).toBe(1);
		expect(byText(body, "button", "Create link")).toBeDefined();
	});

	test("a bot that lives only on this device says a link needs the account", async () => {
		const { container, body } = await mountButton(["calendar"], {
			local: true,
		});
		await click(byText(container, "button", OPEN));
		expect(body.textContent).toContain("needs your account");
		expect(state.reads).toBe(0);
		expect(byText(body, "button", "Create link")).toBeUndefined();
	});
});
