import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import type {
	GateFailure,
	GateReason,
} from "../../../../lib/device-management/model/types";
import type { PreflightRow } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import { appCopy } from "./app-copy";
import {
	type AttentionCopyItem,
	attentionCopy,
	attentionNames,
	defaultCopyTime,
} from "./attention-copy";
import { gateCopy } from "./gate-copy";
import { headlineNames } from "./headline-copy";
import { preflightCopy } from "./preflight-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const time = defaultCopyTime();
const SOON = Math.floor(time.now / 1000) + 6 * 3600;

function item(
	code: AttentionCopyItem["copy"]["code"],
	params: AttentionCopyItem["copy"]["params"],
): AttentionCopyItem {
	return {
		key: code,
		copy: { code, params },
		lastKnown: false,
		firstSeenAt: SOON,
	};
}

describe("attention copy", () => {
	const names = new Map([["usr_mira", "Mira Novak"]]);
	const ctx = { time, personName: (id: string) => names.get(id) };

	test("an account id is never the name of a person (R3, R15)", () => {
		const grant = (person: string) =>
			attentionCopy(
				t,
				item("grant_expiring", {
					device: "edge-berlin-01",
					person,
					expiresAt: SOON,
				}),
				ctx,
			).sentence;
		expect(grant("usr_mira")).toStartWith(
			"Mira Novak's access to edge-berlin-01 ends ",
		);
		expect(grant("usr_9QmT3rVb")).toStartWith(
			"One person's access to edge-berlin-01 ends ",
		);
		expect(grant("usr_9QmT3rVb")).not.toContain("usr_");

		const sandbox = attentionCopy(
			t,
			item("code_running_access_without_sandbox", {
				device: "edge-berlin-01",
				person: "usr_9QmT3rVb",
			}),
			ctx,
		).sentence;
		expect(sandbox).toBe(
			"One person can run code on edge-berlin-01 with the agent's full access.",
		);

		const pending = (owner: string) =>
			attentionCopy(
				t,
				item("access_request_pending", { device: "lab-gpu-02", owner }),
				ctx,
			).sentence;
		expect(pending("usr_mira")).toBe(
			"Waiting for Mira Novak to approve your access to lab-gpu-02.",
		);
		expect(pending("usr_9QmT3rVb")).toBe(
			"Waiting for the owner to approve your access to lab-gpu-02.",
		);
	});

	test("an item names the device and service its sentence sets in mono", () => {
		expect(
			attentionNames(
				item("service_crash_looping", {
					device: "warehouse-pi",
					service: "scanner-ingest",
					ready: 0,
					requested: 1,
				}),
			),
		).toEqual(["warehouse-pi", "scanner-ingest"]);
		expect(attentionNames(item("stale_local_keys", {}))).toEqual([]);
	});
});

describe("headline copy", () => {
	test("a headline names every device and service of its sentences once", () => {
		expect(
			headlineNames({
				code: "fleet.critical",
				params: { count: 2 },
				lists: { names: ["warehouse-pi", "edge-berlin-01"] },
				rest: [
					{
						code: "fleet.coverage",
						params: { readable: 3, active: 5, locked: 1 },
						lists: { locked: ["lab-gpu-02"], events: ["Nightly sync"] },
					},
					{
						code: "fleet.soon_later",
						params: { device: "edge-berlin-01", service: "support-bot" },
					},
				],
			}),
		).toEqual(["warehouse-pi", "edge-berlin-01", "lab-gpu-02", "support-bot"]);
	});
});

describe("gate copy", () => {
	const gate = (
		code: GateReason,
		params: GateFailure["copy"]["params"],
	): GateFailure => ({
		ok: false,
		gate: "G0",
		kind: "live",
		hide: false,
		copy: { code, params },
	});

	test("a live gate names what it gates when the caller says so", () => {
		const device = "warehouse-pi";
		expect(gateCopy(t, gate("offline_needs_live", { device })).inline).toBe(
			"warehouse-pi is offline. This needs a live connection.",
		);
		expect(
			gateCopy(t, gate("offline_needs_live", { device, action: "Start" }))
				.inline,
		).toBe("warehouse-pi is offline. Start needs a live connection.");
		expect(
			gateCopy(
				t,
				gate("offline_needs_live", {
					device,
					action: "Restart and Stop",
					actions: 2,
				}),
			).inline,
		).toBe("warehouse-pi is offline. Restart and Stop need a live connection.");
		expect(
			gateCopy(
				t,
				gate("never_connected_needs_live", { device, action: "Start" }),
			).inline,
		).toBe(
			"warehouse-pi hasn't checked in yet. Start needs a live connection.",
		);
	});
});

describe("app copy", () => {
	test("the newest version isn't called absent while a service's version can't be told", () => {
		const foot = {
			version: "v1.5.0",
			hash: "7c2d1e90",
			when: "today 13:00",
			mode: "online" as const,
			running: 0,
			total: 2,
		};
		const copy = appCopy(t);
		expect(copy.versionFoot(foot)).toContain("isn't running anywhere yet.");
		expect(copy.versionFoot({ ...foot, unknown: 1 })).toBe(
			"Newest version v1.5.0 (7c2d1e90), built today 13:00, isn't on any service whose version is known.",
		);
		expect(copy.versionFoot({ ...foot, running: 1, unknown: 1 })).toContain(
			"runs on 1 of 2 services.",
		);
	});
});

describe("preflight copy", () => {
	test("the clock row says which clocks agree (SPEC §3.10)", () => {
		const row: PreflightRow = {
			id: "D9",
			status: "pass",
			source: "hub",
			copy: { code: "clock_ok" },
		};
		expect(preflightCopy(t, row).text).toBe("This computer and the hub agree");
	});
});
