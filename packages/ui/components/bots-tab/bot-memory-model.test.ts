import { describe, expect, test } from "bun:test";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import {
	DAY_MS,
	groupMemories,
	memoryAnchor,
	memoryFormat,
	memoryPlace,
	memoryTtl,
	memoryValidity,
	placeGlyph,
	wholeDayZone,
} from "./bot-memory-model";

const NOW = new Date(2026, 10, 9, 12, 0).getTime();

function fact(id: string, patch: Partial<BotFact> = {}): BotFact {
	return {
		id,
		text: id,
		kind: "knowledge",
		subject: "user",
		source: "user_stated",
		confidence: 0.9,
		recorded_at: new Date(NOW).toISOString(),
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [],
		...patch,
	};
}

const at = (time: number) => new Date(time).toISOString();

describe("memoryAnchor", () => {
	test("prefers the reference time, then validity start, then the newest evidence", () => {
		const said = NOW - 3 * DAY_MS;
		const evidence = [
			{ seq: 1, message_id: "m1", quote: "", at: at(said - DAY_MS) },
			{ seq: 2, message_id: "m2", quote: "", at: at(said) },
			{ seq: 3, message_id: "m3", quote: "", at: "not a date" },
		];
		expect(memoryAnchor(fact("a", { evidence }))).toBe(said);
		expect(
			memoryAnchor(fact("b", { evidence, valid_from: at(NOW - 9 * DAY_MS) })),
		).toBe(NOW - 9 * DAY_MS);
		expect(
			memoryAnchor(
				fact("c", {
					evidence,
					valid_from: at(NOW - 9 * DAY_MS),
					time: at(NOW + DAY_MS),
				}),
			),
		).toBe(NOW + DAY_MS);
		expect(memoryAnchor(fact("d"))).toBe(NOW);
	});
});

describe("groupMemories", () => {
	test("ranks groups by time whatever order the facts arrive in", () => {
		const groups = groupMemories(
			[
				fact("week", { recorded_at: at(NOW - 4 * DAY_MS) }),
				fact("old", { valid_from: at(new Date(2019, 3, 1).getTime()) }),
				fact("today", { recorded_at: at(NOW - 60_000) }),
				fact("soon", { time: at(NOW + 3 * DAY_MS) }),
				fact("later", { time: at(NOW + 30 * DAY_MS) }),
				fact("yesterday", { recorded_at: at(NOW - DAY_MS) }),
				fact("september", {
					recorded_at: at(new Date(2026, 8, 3).getTime()),
				}),
			],
			NOW,
		);
		expect(groups.map((group) => group.id)).toEqual([
			"upcoming",
			"today",
			"yesterday",
			"week",
			"month-2026-8",
			"year-2019",
		]);
		expect(groups[0].facts.map((item) => item.id)).toEqual(["later", "soon"]);
	});

	test("keeps the backend order inside a group, except upcoming reads soonest first", () => {
		const groups = groupMemories(
			[
				fact("far", { time: at(NOW + 90 * DAY_MS) }),
				fact("near", { time: at(NOW + 2 * DAY_MS) }),
				fact("tea", { recorded_at: at(new Date(2026, 9, 20).getTime()) }),
				fact("city", { recorded_at: at(new Date(2026, 9, 21).getTime()) }),
			],
			NOW,
		);
		expect(groups.map((group) => group.facts.map((item) => item.id))).toEqual([
			["near", "far"],
			["tea", "city"],
		]);
	});

	test("files a date saved at midnight UTC under that calendar day", () => {
		const tomorrow = new Date(NOW);
		const utc = Date.UTC(
			tomorrow.getFullYear(),
			tomorrow.getMonth(),
			tomorrow.getDate() + 1,
		);
		const [group] = groupMemories([fact("x", { time: at(utc) })], NOW);
		expect(group.id).toBe("upcoming");
	});
});

describe("wholeDayZone", () => {
	test("tells dates without a time of day apart from moments", () => {
		expect(wholeDayZone(Date.UTC(2026, 4, 16))).toBe("UTC");
		const localMidnight = new Date(2026, 4, 16).getTime();
		expect(wholeDayZone(localMidnight)).not.toBeNull();
		expect(wholeDayZone(Date.UTC(2026, 4, 16, 14, 30, 7))).toBeNull();
	});
});

describe("memoryTtl", () => {
	test("measures the lifetime left and flags the last day", () => {
		const ttl = memoryTtl(
			fact("t", {
				recorded_at: at(NOW - 3 * DAY_MS),
				expires_at: at(NOW + DAY_MS),
			}),
			NOW,
		);
		expect(ttl?.fraction).toBeCloseTo(0.25);
		expect(ttl?.urgent).toBe(false);
		expect(
			memoryTtl(fact("u", { expires_at: at(NOW + 3_600_000) }), NOW)?.urgent,
		).toBe(true);
		expect(memoryTtl(fact("v"), NOW)).toBeNull();
	});
});

describe("memoryValidity", () => {
	test("reads whether a fact still holds", () => {
		expect(
			memoryValidity(fact("a", { valid_from: at(NOW - DAY_MS) }), NOW)?.state,
		).toBe("current");
		expect(
			memoryValidity(fact("b", { valid_to: at(NOW - DAY_MS) }), NOW)?.state,
		).toBe("ended");
		expect(
			memoryValidity(fact("c", { valid_from: at(NOW + DAY_MS) }), NOW)?.state,
		).toBe("pending");
		expect(memoryValidity(fact("d"), NOW)).toBeNull();
	});
});

describe("memoryPlace", () => {
	test("summarizes each geometry kind with its center and bounds", () => {
		expect(
			memoryPlace({ type: "Point", coordinates: [-9.14, 38.72] }),
		).toMatchObject({ shape: "point", center: [-9.14, 38.72], count: 1 });
		expect(
			memoryPlace({
				type: "MultiPoint",
				coordinates: [
					[9, 53],
					[10, 54],
				],
			}),
		).toMatchObject({ shape: "points", count: 2, center: [9.5, 53.5] });
		expect(
			memoryPlace({
				type: "Polygon",
				coordinates: [
					[
						[0, 0],
						[2, 0],
						[2, 2],
						[0, 0],
					],
				],
			}),
		).toMatchObject({
			shape: "area",
			bounds: [
				[0, 0],
				[2, 2],
			],
		});
		expect(
			memoryPlace({
				type: "GeometryCollection",
				geometries: [
					{ type: "Point", coordinates: [0, 0] },
					{
						type: "LineString",
						coordinates: [
							[0, 0],
							[1, 1],
						],
					},
				],
			}),
		).toMatchObject({ shape: "mixed", count: 2 });
	});

	test("rejects values that aren't usable WGS84 geometries", () => {
		expect(memoryPlace(null)).toBeNull();
		expect(memoryPlace({ type: "Feature", geometry: {} })).toBeNull();
		expect(memoryPlace({ type: "Point", coordinates: [200, 10] })).toBeNull();
		expect(memoryPlace({ type: "Point", coordinates: [] })).toBeNull();
		expect(
			memoryPlace({ type: "GeometryCollection", geometries: [] }),
		).toBeNull();
	});
});

describe("placeGlyph", () => {
	test("centers a single point and draws shapes inside the padding", () => {
		const point = memoryPlace({ type: "Point", coordinates: [10, 50] });
		expect(point && placeGlyph(point).points).toEqual([[20, 20]]);
		const area = memoryPlace({
			type: "Polygon",
			coordinates: [
				[
					[0, 0],
					[4, 0],
					[4, 2],
					[0, 0],
				],
			],
		});
		const glyph = area && placeGlyph(area, 40, 8);
		expect(glyph?.areas.startsWith("M8 ")).toBe(true);
		expect(glyph?.areas.endsWith("Z")).toBe(true);
		const coordinates = (glyph?.areas.match(/[\d.]+/g) ?? []).map(Number);
		expect(Math.min(...coordinates)).toBeGreaterThanOrEqual(8);
		expect(Math.max(...coordinates)).toBeLessThanOrEqual(32);
	});

	test("draws a repeated point once", () => {
		const place = memoryPlace({
			type: "MultiPoint",
			coordinates: [
				[1, 1],
				[1, 1],
				[2, 2],
			],
		});
		expect(place && placeGlyph(place).points).toHaveLength(2);
	});
});

describe("memoryFormat", () => {
	test("speaks of whole days by calendar day", () => {
		const format = memoryFormat("en");
		const today = new Date(NOW);
		const inTwoDays = Date.UTC(
			today.getFullYear(),
			today.getMonth(),
			today.getDate() + 2,
		);
		expect(format.relative(inTwoDays, NOW)).toBe("in 2 days");
		expect(format.relative(NOW + 3 * 3_600_000 + 7_000, NOW)).toBe(
			"in 3 hours",
		);
		expect(format.dayOfMonth(Date.UTC(2026, 4, 16))).toBe("16");
	});
});
