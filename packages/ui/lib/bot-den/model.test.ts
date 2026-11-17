import { describe, expect, it } from "bun:test";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	FLUX_DEN_KEY,
	denActivities,
	denCenter,
	denCrew,
	denLength,
	isArea,
	perchOn,
	perchShare,
	sideDenShape,
} from "./model";

function bot(id: string, extra: Partial<IBot> = {}): IBot {
	return {
		id,
		system: null,
		name: id,
		capabilities: ["flowpilot"],
		appearance: { kind: "flux" },
		...extra,
	} as IBot;
}

const flux = { name: "Flux", appearance: { kind: "flux" as const } };
const nameOf = (item: IBot) => item.name ?? item.id;

describe("den crew", () => {
	it("holds Flux first, then the builders the profile shows", () => {
		const crew = denCrew(
			[
				bot("flux-row", { system: "flux" }),
				bot("copy"),
				bot("hidden-copy"),
				bot("pip", { capabilities: ["internet", "computer_use"] }),
			],
			new Set(["hidden-copy"]),
			flux,
			nameOf,
		);
		expect(crew.map((item) => [item.key, item.chatBotId])).toEqual([
			[FLUX_DEN_KEY, null],
			["copy", "copy"],
		]);
	});
});

describe("den activities", () => {
	it("shows Flux working on a Flux run and a bot asking for its open prompt", () => {
		const activities = denActivities(
			[FLUX_DEN_KEY, "copy", "idle"],
			{ r1: { conversationId: "session-1" } },
			{ conversationId: "bot:copy" },
		);
		expect(activities.get(FLUX_DEN_KEY)?.agentState).toBe("working");
		expect(activities.get("copy")?.agentState).toBe("needs-you");
		expect(activities.get("idle")?.agentState).toBe("idle");
	});
});

describe("den placement", () => {
	const bounds = { right: 1200, top: 80, bottom: 900 };

	it("rests low on its edge", () => {
		const length = denLength(2, false);
		const center = denCenter(bounds, length, []);
		expect(center).not.toBeNull();
		expect((center ?? 0) + length / 2).toBeCloseTo(900 - 132, 5);
	});

	it("moves above the chat overlay that covers its edge", () => {
		const length = denLength(1, false);
		const overlay = { x: 736, y: 236, width: 440, height: 640 };
		const center = denCenter(bounds, length, [overlay]);
		expect((center ?? 0) + length / 2).toBeLessThanOrEqual(236 - 16);
	});

	it("ignores what does not reach its edge", () => {
		const length = denLength(1, false);
		const panel = { x: 100, y: 300, width: 400, height: 600 };
		expect(denCenter(bounds, length, [panel])).toBe(
			denCenter(bounds, length, []),
		);
	});

	it("tucks away when the overlay leaves it no room", () => {
		const length = denLength(3, false);
		const overlay = { x: 736, y: 90, width: 440, height: 800 };
		expect(denCenter(bounds, length, [overlay])).toBeNull();
	});

	it("stacks its bots along the right edge", () => {
		const den = sideDenShape(1200, 600, ["a", "b"], false);
		expect(den.edge).toBe("right");
		expect(den.rect.x + den.rect.width).toBe(1200);
		expect(den.slots.map((slot) => slot.y)).toEqual([584, 616]);
		expect(den.slots[0].x).toBeCloseTo(1184.5, 5);
	});
});

describe("perching", () => {
	const node = { x: 300, y: 400, width: 200, height: 80 };
	const board = { x: 60, y: 40, width: 1100, height: 820 };

	it("keeps a dropped bot off the target's very ends", () => {
		expect(perchShare(node, { x: 290, y: 410 }).x).toBeCloseTo(0.12, 5);
		expect(perchShare(node, { x: 450, y: 410 }).x).toBeCloseTo(0.75, 5);
	});

	it("stands on a small target's top edge, or inside when that edge is too high", () => {
		expect(perchOn(node, { x: 0.5, y: 0.5 }, 80)).toMatchObject({
			x: 400,
			edgeY: 400,
		});
		const high = perchOn({ ...node, y: 90 }, { x: 0.5, y: 0.5 }, 80);
		expect(high.edgeY).toBeNull();
		expect(high.y).toBeGreaterThan(90);
	});

	it("stays where it was dropped in an area like the board", () => {
		expect(isArea(board)).toBe(true);
		expect(isArea(node)).toBe(false);
		const spot = perchOn(board, perchShare(board, { x: 560, y: 690 }), 44);
		expect(spot.x).toBeCloseTo(560, 5);
		expect(spot.y).toBeCloseTo(690, 5);
		expect(spot.edgeY).toBeNull();
	});
});
