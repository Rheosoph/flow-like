import { describe, expect, test } from "bun:test";
import {
	FRESHNESS_RULES,
	baseSource,
	classify,
	sameSource,
	worstPlane,
} from "./freshness";
import type { Freshness, FreshnessSignal } from "./types";

const NOW = 1_790_000_000;
const ago = (signal: FreshnessSignal, ageS: number, sessionOpen?: boolean) =>
	classify(signal, { now: NOW, at: NOW - ageS, loaded: true, sessionOpen }).age;

describe("source tags", () => {
	test.each([
		["device_row", "hub"],
		["presence", "hub"],
		["cert_inventory", "hub"],
		["fleet_status", "snap"],
		["fleet_metrics", "snap"],
		["saved_inventory", "saved"],
		["live_inspection", "live"],
		["local_vault", "local"],
		["agent_local", "device"],
	] as const)("%s comes from %s", (signal, src) => {
		expect(FRESHNESS_RULES[signal].src).toBe(src);
		expect(classify(signal, { now: NOW, loaded: false }).src).toBe(src);
	});
});

describe("age boundaries", () => {
	test("presence: current ≤ 2 min, delayed to 10 min, then last known", () => {
		expect(ago("presence", 120)).toBe("current");
		expect(ago("presence", 121)).toBe("delayed");
		expect(ago("presence", 600)).toBe("delayed");
		expect(ago("presence", 601)).toBe("lastknown");
	});

	test("fleet status: current ≤ 90 s, delayed to 10 min, then last known", () => {
		expect(ago("fleet_status", 90)).toBe("current");
		expect(ago("fleet_status", 91)).toBe("delayed");
		expect(ago("fleet_status", 600)).toBe("delayed");
		expect(ago("fleet_status", 601)).toBe("lastknown");
	});

	test("fleet metrics: current ≤ 75 s, then last known (no forced stale)", () => {
		expect(ago("fleet_metrics", 0)).toBe("current");
		expect(ago("fleet_metrics", 75)).toBe("current");
		expect(ago("fleet_metrics", 76)).toBe("lastknown");
	});

	test("live inspection: live while the session is open and ≤ 30 s", () => {
		expect(ago("live_inspection", 30, true)).toBe("live");
		expect(ago("live_inspection", 31, true)).toBe("lastknown");
		expect(ago("live_inspection", 5, false)).toBe("lastknown");
	});

	test("live metrics: live ≤ 15 s", () => {
		expect(ago("live_metrics", 15, true)).toBe("live");
		expect(ago("live_metrics", 16, true)).toBe("lastknown");
	});

	test("certificate inventory: fresh ≤ 2 h; null updated_at = never reported", () => {
		expect(ago("cert_inventory", 7_200)).toBe("current");
		expect(ago("cert_inventory", 7_201)).toBe("lastknown");
		const never = classify("cert_inventory", {
			now: NOW,
			at: null,
			loaded: true,
		});
		expect(never.age).toBe("notloaded");
		expect(never.reason).toEqual({ code: "never_reported" });
	});

	test("saved inventory is always a snapshot", () => {
		expect(ago("saved_inventory", 0)).toBe("snapshot");
		expect(ago("saved_inventory", 30 * 86_400)).toBe("snapshot");
	});

	test("hub rows without thresholds stay current while they load", () => {
		expect(ago("device_row", 3_600)).toBe("current");
		expect(classify("hub_support", { now: NOW, loaded: true }).age).toBe(
			"current",
		);
	});
});

describe("states that are never empty (R6)", () => {
	test("not loaded, locked, no access, unsupported and error are distinct", () => {
		const states = [
			classify("fleet_status", { now: NOW, loaded: false }),
			classify("fleet_status", { now: NOW, loaded: true, locked: true }),
			classify("fleet_status", {
				now: NOW,
				loaded: false,
				noAccess: { code: "needs_capability" },
			}),
			classify("live_inspection", {
				now: NOW,
				loaded: false,
				unsupported: { code: "agent_update_needed" },
			}),
			classify("fleet_status", {
				now: NOW,
				loaded: false,
				error: { code: "network" },
			}),
		].map((freshness) => freshness.age);
		expect(states).toEqual([
			"notloaded",
			"locked",
			"noaccess",
			"unsupported",
			"error",
		]);
	});

	test("an error after a success keeps the data time", () => {
		const failed = classify("fleet_status", {
			now: NOW,
			at: NOW - 40,
			loaded: true,
			error: { code: "network", retryAt: NOW + 27 },
		});
		expect(failed).toMatchObject({
			age: "error",
			at: NOW - 40,
			dataFrom: NOW - 40,
			error: { code: "network", retryAt: NOW + 27 },
		});
	});

	test("a first-load error has no data time", () => {
		const failed = classify("device_row", {
			now: NOW,
			loaded: false,
			error: { code: "timeout" },
		});
		expect(failed.age).toBe("error");
		expect(failed.dataFrom).toBeUndefined();
	});

	test("locked keeps the last read time", () => {
		expect(
			classify("fleet_status", {
				now: NOW,
				at: NOW - 600,
				loaded: true,
				locked: true,
			}),
		).toMatchObject({ age: "locked", at: NOW - 600 });
	});
});

describe("clock skew", () => {
	test("a negative age is flagged, not hidden", () => {
		const skewed = classify("fleet_status", {
			now: NOW,
			at: NOW + 240,
			loaded: true,
		});
		expect(skewed.age).toBe("current");
		expect(skewed.skewS).toBe(-240);
	});

	test("known skew is flagged only above 120 s", () => {
		const input = { now: NOW, at: NOW - 10, loaded: true };
		expect(
			classify("fleet_status", { ...input, skewS: 120 }).skewS,
		).toBeUndefined();
		expect(classify("fleet_status", { ...input, skewS: 420 }).skewS).toBe(420);
	});
});

describe("hub-failing override (R5)", () => {
	const hubFailing = {
		error: { code: "timeout" as const, retryAt: NOW + 30 },
		dataFrom: NOW - 90,
	};

	test("every Hub stamp reads couldn't refresh with data from the last success", () => {
		for (const signal of Object.keys(FRESHNESS_RULES) as FreshnessSignal[]) {
			if (FRESHNESS_RULES[signal].src !== "hub") continue;
			const failed = classify(signal, {
				now: NOW,
				at: NOW - 5,
				loaded: true,
				hubFailing,
			});
			expect(failed.age).toBe("error");
			expect(failed.dataFrom).toBe(NOW - 90);
			expect(failed.error?.code).toBe("timeout");
		}
	});

	test("non-hub planes and noFail rows keep their age", () => {
		expect(
			classify("fleet_status", {
				now: NOW,
				at: NOW - 5,
				loaded: true,
				hubFailing,
			}).age,
		).toBe("current");
		expect(
			classify("cert_inventory", {
				now: NOW,
				at: NOW - 60,
				loaded: true,
				hubFailing,
				noFail: true,
			}).age,
		).toBe("current");
	});
});

describe("baseSource / sameSource / worstPlane", () => {
	const hub: Freshness = { src: "hub", age: "current", at: NOW - 5 };
	const hubOlder: Freshness = { src: "hub", age: "current", at: NOW - 50 };
	const snap: Freshness = { src: "snap", age: "delayed", at: NOW - 200 };

	test("the block head is the source shared by at least two rows, dated by the oldest", () => {
		expect(baseSource([hub, snap, hubOlder])).toBe(hubOlder);
		expect(baseSource([hub, snap])).toBeUndefined();
		expect(baseSource([])).toBeUndefined();
	});

	test("Hub wins a tie", () => {
		const snap2: Freshness = { ...snap, at: NOW - 300 };
		expect(baseSource([snap, snap2, hub, hubOlder])?.src).toBe("hub");
	});

	test("rows that repeat the head drop their stamp", () => {
		expect(sameSource(hubOlder, hub)).toBe(true);
		expect(sameSource(snap, hub)).toBe(false);
		expect(sameSource(hub, undefined)).toBe(false);
		expect(
			sameSource({ ...hub, age: "error", error: { code: "network" } }, hub),
		).toBe(false);
	});

	test("the worst plane is the most degraded age; ties go to plane order", () => {
		expect(
			worstPlane({
				hub,
				snap,
				live: { src: "live", age: "live" },
				local: { src: "local", age: "current" },
			}),
		).toBe("snap");
		expect(
			worstPlane({
				hub: { src: "hub", age: "error", error: { code: "network" } },
				snap,
			}),
		).toBe("hub");
		expect(worstPlane({ hub, local: { src: "local", age: "current" } })).toBe(
			"hub",
		);
		expect(worstPlane({})).toBeUndefined();
	});
});
