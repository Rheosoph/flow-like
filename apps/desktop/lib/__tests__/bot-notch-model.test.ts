import type { IBot } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { describe, expect, it } from "vitest";
import {
	acceptsTarget,
	botActivity,
	denCalledOut,
	denShape,
	notchBotsOf,
	notchEligible,
	perchAnchor,
	perchCenter,
	pointedAtOf,
	previewText,
	shownInNotch,
	targetLabel,
} from "../bot-notch/model";
import type { HostTurn, NotchLayout, NotchWindow } from "../bot-notch/protocol";

function bot(overrides: Partial<IBot>): IBot {
	return {
		id: "bot_a",
		owner_id: "owner",
		system: null,
		preset_id: null,
		name: "A",
		description: null,
		instructions: null,
		appearance: { kind: "pip" },
		model: { primary: null, fallbacks: [] },
		classification: { level: "internal", pii: false },
		capabilities: ["computer_use"],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: "",
		updated_at: "",
		...overrides,
	} as IBot;
}

const plainScreen: NotchLayout = {
	width: 1600,
	height: 1000,
	scale: 2,
	menuBarHeight: 24,
	notch: null,
	fullscreen: false,
	platform: "macos",
};

const notchedScreen: NotchLayout = {
	...plainScreen,
	menuBarHeight: 37,
	notch: { left: 700, right: 900, height: 37 },
};

const chrome: NotchWindow = {
	id: "4711",
	pid: 9,
	app: "Google Chrome",
	title: "Pull requests · flow-like",
	bounds: { x: 100, y: 200, width: 800, height: 600 },
	browser: "chrome",
};

const editor: NotchWindow = {
	...chrome,
	id: "12",
	app: "Code",
	title: "main.rs — flow-like",
	browser: null,
};

function turn(overrides: Partial<HostTurn>): HostTurn {
	return {
		id: "bt_1",
		bot_id: "bot_a",
		conversation_id: "c",
		trigger_message_id: "m",
		status: "running",
		...overrides,
	};
}

describe("notch crew", () => {
	it("takes bots on this computer that see the screen or drive a browser", () => {
		expect(notchEligible(bot({}))).toBe(true);
		expect(notchEligible(bot({ capabilities: ["browser_use"] }))).toBe(true);
		expect(notchEligible(bot({ capabilities: ["internet"] }))).toBe(false);
		expect(notchEligible(bot({ location: "server" }))).toBe(false);
		expect(notchEligible(bot({ location: "device" }))).toBe(false);
		expect(notchEligible(bot({ location: "server", local: true }))).toBe(true);
	});

	it("leaves out bots the profile or the notch hides, by visibility key", () => {
		const bots = [
			bot({ id: "bot_a" }),
			bot({ id: "bot_b", system: "general", capabilities: ["browser_use"] }),
			bot({ id: "bot_c" }),
		];
		const listed = notchBotsOf(bots, new Set(["bot_c"]), (b) => b.id);
		expect(listed.map((b) => b.key)).toEqual(["bot_a", "general"]);
		expect(listed[1]).toMatchObject({ computer: false, browser: true });
		expect(shownInNotch(listed, ["general"]).map((b) => b.id)).toEqual([
			"bot_a",
		]);
	});

	it("lets a window go to a screen bot and a tab to either kind", () => {
		const screen = { computer: true, browser: false };
		const browser = { computer: false, browser: true };
		expect(acceptsTarget(screen, editor)).toBe(true);
		expect(acceptsTarget(browser, editor)).toBe(false);
		expect(acceptsTarget(browser, chrome)).toBe(true);
		expect(acceptsTarget(screen, chrome)).toBe(true);
	});
});

describe("targets", () => {
	it("points at a tab with its address and at a window without one", () => {
		expect(
			pointedAtOf(
				chrome,
				{ url: "https://github.com/x/pulls", title: "Pulls" },
				"desktop:1",
			),
		).toEqual({
			kind: "tab",
			host_id: "desktop:1",
			window_id: "4711",
			app: "Google Chrome",
			title: "Pulls",
			url: "https://github.com/x/pulls",
		});
		expect(pointedAtOf(editor, null, "desktop:1")).toEqual({
			kind: "window",
			host_id: "desktop:1",
			window_id: "12",
			app: "Code",
			title: "main.rs — flow-like",
		});
	});

	it("labels a tab by title and host, a window by app and title", () => {
		expect(
			targetLabel(chrome, { url: "https://github.com/x", title: "Repo" }),
		).toBe("Repo · github.com");
		expect(targetLabel(chrome, null)).toBe("Pull requests · flow-like");
		expect(targetLabel(editor, null)).toBe("Code — main.rs — flow-like");
		expect(targetLabel({ ...editor, title: null }, null)).toBe("Code");
	});
});

describe("den", () => {
	it("centres a pill on a plain screen with one slot per bot", () => {
		const den = denShape(plainScreen, ["a", "b", "c"], false);
		expect(den.wrapsNotch).toBe(false);
		expect(den.rect.x + den.rect.width / 2).toBe(800);
		expect(den.rect.height).toBe(30);
		expect(den.slots.map((slot) => slot.x)).toEqual([768, 800, 832]);
	});

	it("puts bots beside the hardware notch, never under it", () => {
		const den = denShape(notchedScreen, ["a", "b", "c"], false);
		expect(den.wrapsNotch).toBe(true);
		for (const slot of den.slots)
			expect(slot.x < 700 - 12 || slot.x > 900 + 12).toBe(true);
		expect(den.slots[0].x).toBeGreaterThan(900);
		expect(den.slots[1].x).toBeLessThan(700);
		expect(den.rect.x + den.rect.width / 2).toBe(800);
	});

	it("grows downward on hover and keeps the bots where they were", () => {
		const rest = denShape(plainScreen, ["a"], false);
		const hover = denShape(plainScreen, ["a"], true);
		expect(hover.rect.height).toBeGreaterThan(rest.rect.height);
		expect(hover.slots[0].y).toBe(rest.slots[0].y);
	});

	it("lies flat in the top edge when tucked, its bots above the screen", () => {
		for (const screen of [plainScreen, notchedScreen]) {
			const rest = denShape(screen, ["a", "b"], false);
			const tucked = denShape(screen, ["a", "b"], true, true);
			expect(tucked.rect.height).toBe(0);
			expect(tucked.rect.x).toBe(rest.rect.x);
			expect(tucked.rect.width).toBe(rest.rect.width);
			expect(tucked.slots.map((slot) => slot.x)).toEqual(
				rest.slots.map((slot) => slot.x),
			);
			for (const slot of tucked.slots) expect(slot.y).toBeLessThan(0);
		}
	});

	it("is called out by the pointer, a gesture or a bot at home, never by a bot on a window", () => {
		const perched = new Set(["on_window"]);
		expect(denCalledOut([false, false], [], perched)).toBe(false);
		expect(denCalledOut([false, true], [], perched)).toBe(true);
		expect(denCalledOut([], ["at_home"], perched)).toBe(true);
		expect(denCalledOut([], [null], perched)).toBe(true);
		expect(denCalledOut([], ["on_window", undefined], perched)).toBe(false);
	});
});

describe("perches", () => {
	it("sits under the drop, clear of the window buttons and other bots", () => {
		const bounds = { x: 100, y: 200, width: 800, height: 600 };
		expect(perchAnchor(bounds, 120, [])).toBe(84);
		expect(perchAnchor(bounds, 500, [])).toBe(400);
		expect(perchAnchor(bounds, 500, [400])).toBe(446);
		expect(perchAnchor(bounds, 2000, [])).toBe(764);
		expect(perchAnchor(bounds, 2000, [764])).toBe(718);
	});

	it("stands on the top edge, or inside it when the edge is under the menu bar", () => {
		const free = perchCenter(
			{ x: 0, y: 300, width: 500, height: 400 },
			100,
			26,
		);
		expect(free.y).toBeLessThan(300);
		const high = perchCenter({ x: 0, y: 25, width: 500, height: 400 }, 100, 26);
		expect(high.y).toBeGreaterThan(26);
	});
});

describe("activity", () => {
	it("prefers the live turn, then one that just ended", () => {
		const now = Date.parse("2026-11-17T12:00:10Z");
		const done = turn({
			id: "bt_0",
			status: "done",
			finished_at: "2026-11-17T12:00:05Z",
		});
		expect(botActivity([done], "bot_a", now).agentState).toBe("done");
		expect(
			botActivity([done, turn({ status: "waiting" })], "bot_a", now).agentState,
		).toBe("needs-you");
		expect(botActivity([turn({ status: "queued" })], "bot_a", now).mood).toBe(
			"thinking",
		);
		expect(botActivity([done], "bot_a", now + 60_000).agentState).toBe("idle");
		expect(botActivity([done], "bot_b", now).turn).toBeNull();
	});
});

describe("reply preview", () => {
	it("keeps the words and drops markdown and code", () => {
		expect(
			previewText(
				"**Done.** I fixed [the bug](https://x.y).\n```ts\nconst a = 1;\n```\nAll green.",
			),
		).toBe("Done. I fixed the bug. All green.");
		expect(previewText("Ran computer_screenshot and _kept_ going")).toBe(
			"Ran computer_screenshot and kept going",
		);
		expect(previewText("x".repeat(400), 10)).toBe(`${"x".repeat(9)}…`);
	});
});
