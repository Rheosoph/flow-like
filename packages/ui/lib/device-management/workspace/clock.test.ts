import { describe, expect, test } from "bun:test";
import { ADMISSION_TTL_S, admissionHubTimeS, createClockModel } from "./clock";
import type { ClockModel } from "./types";

const HUB = 1_727_770_000;

function fixture(startMs = HUB * 1_000) {
	let local = startMs;
	const clock = createClockModel({ now: () => local });
	return {
		clock,
		advance: (ms: number) => {
			local += ms;
		},
		local: () => local,
	};
}

describe("clock model", () => {
	test("without samples it is this computer's clock and knows no offset", () => {
		const { clock, local } = fixture();
		expect(clock.now()).toBe(local());
		expect(clock.hubOffsetS).toBeUndefined();
		expect(clock.deviceSkewS("d")).toBeUndefined();
	});

	test("server_time corrects now() by the lowest recent offset", () => {
		const { clock, local } = fixture((HUB + 420) * 1_000);
		clock.observe("server_time", HUB, local() + 900);
		clock.observe("server_time", HUB, local() + 200);
		clock.observe("server_time", HUB, local() + 600);
		expect(clock.hubOffsetS).toBeCloseTo(420.2, 5);
		expect(clock.now()).toBe(local() - 420_200);
	});

	test("a later clock fix replaces the old offset at once in both directions", () => {
		const ahead = fixture();
		ahead.clock.observe("server_time", HUB - 600, ahead.local());
		expect(ahead.clock.hubOffsetS).toBe(600);
		ahead.advance(30_000);
		ahead.clock.observe("server_time", HUB + 30, ahead.local());
		expect(ahead.clock.hubOffsetS).toBe(0);

		const behind = fixture();
		behind.clock.observe("server_time", HUB + 600, behind.local());
		expect(behind.clock.hubOffsetS).toBe(-600);
		behind.advance(30_000);
		behind.clock.observe("server_time", HUB + 30, behind.local());
		expect(behind.clock.hubOffsetS).toBe(0);
	});

	test("old samples leave the window", () => {
		const { clock, advance, local } = fixture();
		clock.observe("server_time", HUB, local() + 1_000);
		advance(11 * 60_000);
		clock.observe("server_time", HUB + 660, local() + 3_000);
		expect(clock.hubOffsetS).toBeCloseTo(3, 5);
	});

	test("the admission heuristic is a fallback that server_time overrides", () => {
		expect(admissionHubTimeS(HUB + ADMISSION_TTL_S)).toBe(HUB);
		const { clock, local } = fixture((HUB + 120) * 1_000);
		clock.observe("admission", admissionHubTimeS(HUB + 300), local());
		expect(clock.hubOffsetS).toBe(120);
		clock.observe("server_time", HUB + 118, local());
		expect(clock.hubOffsetS).toBe(2);
	});

	test("device skew comes from attributed snapshots, newest-looking sample first, against hub time", () => {
		const { clock, advance, local } = fixture((HUB + 10) * 1_000);
		clock.observe("server_time", HUB, local());
		clock.observe("snapshot", HUB + 300, local(), "d1");
		clock.observe("snapshot", HUB + 240, local(), "d1");
		clock.observe("snapshot", HUB - 999, local());
		expect(clock.deviceSkewS("d1")).toBe(300);
		expect(clock.deviceSkewS("d2")).toBeUndefined();
		advance(11 * 60_000);
		clock.observe("snapshot", HUB + 660 - 5, local(), "d1");
		expect(clock.deviceSkewS("d1")).toBe(-5);
	});

	test("ignores non-finite samples and satisfies the ClockModel contract", () => {
		const { clock, local } = fixture();
		clock.observe("server_time", Number.NaN, local());
		clock.observe("server_time", HUB, Number.POSITIVE_INFINITY);
		expect(clock.hubOffsetS).toBeUndefined();
		const contract: ClockModel = clock;
		contract.observe("admission", HUB, local());
		expect(contract.hubOffsetS).toBe(0);
	});
});
