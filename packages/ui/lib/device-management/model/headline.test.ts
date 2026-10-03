import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import type { CopyTime } from "../../../components/settings/devices/copy/attention-copy";
import {
	headlineCopy,
	headlinePartCopy,
} from "../../../components/settings/devices/copy/headline-copy";
import type { DevicesT } from "../../../components/settings/devices/primitives/area-context";
import { formatMoment, formatRelativeTime } from "../../date";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_NOW,
	placement,
	sampleFleet,
} from "./__fixtures__/sample-fleet";
import { generateFleet } from "./__fixtures__/sample-fleet-200";
import {
	type AttentionInputExt,
	computeAttention,
	createAttentionMemory,
} from "./attention";
import { attentionCandidate } from "./device-view";
import { classify } from "./freshness";
import { type HeadlineApp, headline } from "./headline";
import type { AttentionItem, PlacementStatusPlus } from "./types";

const ID = SAMPLE_IDS;
const APP = SAMPLE_APPS;
const t = getI18n().getFixedT("en", "devices") as DevicesT;
const NOW_MS = SAMPLE_NOW * 1000;
const time: CopyTime = {
	now: NOW_MS,
	locale: "en-GB",
	ago: (atS, style = "long") =>
		formatRelativeTime(atS * 1000, style, "", { now: NOW_MS, locale: "en" }),
	at: (atS) =>
		formatMoment(atS * 1000, {
			now: NOW_MS,
			locale: "en-GB",
			timeZone: "Europe/Berlin",
		}),
};

function itemsOf(input: AttentionInputExt) {
	return computeAttention(input, createAttentionMemory("test", undefined));
}

function fleetText(
	input: AttentionInputExt,
	items: readonly AttentionItem[] = itemsOf(input),
) {
	return headlineCopy(t, headline(input, { items }), time);
}

function app(
	appId: string,
	name: string,
	patch: Partial<HeadlineApp> = {},
): HeadlineApp {
	return {
		appId,
		name,
		localOnly: false,
		events: { total: 2, eligible: 2, nowhere: [] },
		...patch,
	};
}

function appHeadline(input: AttentionInputExt, entry: HeadlineApp) {
	return headline(input, { items: itemsOf(input), app: entry });
}

function appText(input: AttentionInputExt, entry: HeadlineApp) {
	return headlineCopy(t, appHeadline(input, entry), time);
}

/** Patch one placement of a live device's inspection. */
function withLivePlacement(
	deviceId: string,
	serviceId: string,
	patch: Partial<PlacementStatusPlus>,
	input = sampleFleet(),
) {
	const inspection = input.live[deviceId].inspection;
	if (!inspection) throw new Error(`fixture: ${deviceId} inspection`);
	const placements = inspection.value.placements;
	const index = placements.findIndex((entry) => entry.id === serviceId);
	if (index < 0) throw new Error(`fixture: no ${serviceId}`);
	placements[index] = { ...placements[index], ...patch };
	return input;
}

/** Add a placement to the studio's live inspection. */
function onStudio(row: PlacementStatusPlus, input = sampleFleet()) {
	input.live[ID.studio].inspection?.value.placements.push(row);
	return input;
}

function criticalOn(subject: AttentionItem["subject"]): AttentionItem {
	return {
		...attentionCandidate({
			key: "pending_setup_expired",
			severity: "critical",
			subject,
			source: classify("device_row", { now: SAMPLE_NOW, loaded: true }),
		}),
		firstSeenAt: SAMPLE_NOW,
	};
}

describe("fleet headline (SPEC §4.24, §6.4)", () => {
	test("golden sample reads exactly as SPEC §4.24", () => {
		const input = sampleFleet();
		expect(headline(input, { items: itemsOf(input) })).toEqual({
			code: "fleet.critical",
			params: { count: 1 },
			lists: { names: ["warehouse-pi"] },
			rest: [
				{ code: "fleet.soon_later", params: { soon: 8, later: 5 } },
				{
					code: "fleet.coverage",
					params: { readable: 3, active: 5, locked: 2 },
					lists: { locked: ["lab-gpu-02", "cold-storage-nas"] },
				},
			],
		});
		expect(fleetText(input)).toEqual({
			lead: "warehouse-pi needs you now.",
			rest: "8 things need a look soon and 5 can wait. Status comes from 3 of 5 active devices; lab-gpu-02 and cold-storage-nas are locked.",
		});
	});

	test("critical device names cap at 2, then 'and N others'", () => {
		const input = sampleFleet();
		const items = [
			...itemsOf(input),
			criticalOn({ kind: "device", deviceId: ID.edge }),
			criticalOn({ kind: "device", deviceId: ID.studio }),
			criticalOn({ kind: "device", deviceId: ID.cold }),
		];
		expect(fleetText(input, items).lead).toBe(
			"edge-berlin-01, warehouse-pi and 2 others need you now.",
		);
	});

	test("criticals without a device name the count", () => {
		const input = sampleFleet();
		const items = [criticalOn({ kind: "hub" }), criticalOn({ kind: "setup" })];
		expect(headline(input, { items })).toMatchObject({
			code: "fleet.critical_items",
			params: { count: 2 },
		});
		expect(fleetText(input, items).lead).toBe("2 things need you now.");
	});

	test("no criticals: 'Nothing is broken.'", () => {
		const input = sampleFleet();
		const items = itemsOf(input).filter((item) => item.severity !== "critical");
		expect(fleetText(input, items).lead).toBe("Nothing is broken.");
		const noticesOnly = items.filter((item) => item.severity !== "warning");
		expect(fleetText(input, noticesOnly).rest).toStartWith(
			"Nothing needs a look soon and 5 can wait.",
		);
		const warningsOnly = items.filter((item) => item.severity !== "notice");
		expect(fleetText(input, warningsOnly).rest).toStartWith(
			"8 things need a look soon. ",
		);
	});

	test("nothing at all, or Info only: 'Everything runs as you asked.' + coverage", () => {
		const input = sampleFleet();
		const info = itemsOf(input).filter((item) => item.severity === "info");
		expect(info.length).toBeGreaterThan(0);
		for (const items of [[], info])
			expect(fleetText(input, items)).toEqual({
				lead: "Everything runs as you asked.",
				rest: "Status comes from 3 of 5 active devices; lab-gpu-02 and cold-storage-nas are locked.",
			});
	});

	test("coverage without locked devices drops the clause", () => {
		const input = sampleFleet();
		input.devices = input.devices.filter(
			(row) => row.device_id !== ID.lab && row.device_id !== ID.cold,
		);
		expect(fleetText(input, []).rest).toBe(
			"Status comes from 3 of 3 active devices.",
		);
	});

	test("a generated fleet of 200 still reads as one sentence set", () => {
		const { input } = generateFleet(200);
		const { lead, rest } = fleetText(input);
		expect(lead).toMatch(/ and \d+ others need you now\.$/);
		expect(rest).toMatch(/^\d+ things need a look soon and \d+ can wait\. /);
		expect(`${lead} ${rest}`).not.toContain("{{");
	});
});

describe("app headline (APP §7.4, first match wins)", () => {
	test("no devices", () => {
		const input = sampleFleet();
		input.devices = [];
		expect(appText(input, app(APP.invoiceAi, "Invoice AI"))).toEqual({
			lead: "You don't have any devices yet.",
			rest: "Set one up, then deploy Invoice AI to it.",
		});
	});

	test("all locked: a device that never checked in is named, not counted as locked", () => {
		const input = sampleFleet();
		for (const keys of input.keys) keys.state = "locked";
		expect(appText(input, app(APP.invoiceAi, "Invoice AI"))).toEqual({
			lead: "Unlock to see where Invoice AI runs.",
			rest: "The hub doesn't know which apps run on your devices; only keys on this computer can read it. 4 devices are locked. cold-storage-nas hasn't checked in yet.",
		});
	});

	test("only devices that never checked in: nothing to unlock, nothing uncertain", () => {
		const input = sampleFleet();
		input.devices = input.devices.filter((row) => row.device_id === ID.cold);
		expect(appText(input, app(APP.invoiceAi, "Invoice AI"))).toEqual({
			lead: "Invoice AI isn't on any device yet.",
			rest: "It's an online app, so devices will run it online with its data in the cloud. 2 of its 2 events can run on a device.",
		});
	});

	test("never deployed with a device shared for another app still says 'you can see'", () => {
		const input = sampleFleet();
		input.devices = input.devices.filter((row) => row.device_id !== ID.cold);
		expect(
			appText(input, app(APP.partnerReports, "Partner Reports")).lead,
		).toBe("Partner Reports isn't on any device you can see yet.");
	});

	test("never deployed: online and local-only", () => {
		const input = sampleFleet();
		const reports = app(APP.partnerReports, "Partner Reports", {
			events: { total: 3, eligible: 2, nowhere: [] },
		});
		expect(appText(input, reports)).toEqual({
			lead: "Partner Reports isn't on any device you can see yet.",
			rest: "It's an online app, so devices will run it online with its data in the cloud. 2 of its 3 events can run on a device.",
		});
		const unlocked = sampleFleet();
		unlocked.devices = unlocked.devices.filter(
			(row) => row.device_id !== ID.lab && row.device_id !== ID.cold,
		);
		expect(appText(unlocked, { ...reports, localOnly: true })).toEqual({
			lead: "Partner Reports isn't on any device yet.",
			rest: "It's a local-only app, so devices will get an offline copy from this computer. 2 of its 3 events can run on a device.",
		});
	});

	test("crashing, last known: the device is offline", () => {
		expect(
			appText(sampleFleet(), app(APP.warehouseScanner, "Warehouse Scanner")),
		).toEqual({
			lead: "scanner-ingest on warehouse-pi kept crashing when last seen.",
			rest: "warehouse-pi has been offline since 11:00 (3 hours ago), so this is the last known state. Status from 3 of 5 devices you can see; 1 hasn't checked in yet and your access doesn't cover Warehouse Scanner on 1 more.",
		});
	});

	test("crashing, live: names the services that are fine", () => {
		const input = onStudio(
			placement({
				id: "support-bot",
				project_id: APP.supportPortal,
				desired_state: "running",
				observed_state: "running",
			}),
			withLivePlacement(ID.edge, "support-bot", { observed_state: "backoff" }),
		);
		expect(appText(input, app(APP.supportPortal, "Support Portal"))).toEqual({
			lead: "support-bot on edge-berlin-01 keeps crashing.",
			rest: "1 other service runs as you asked. Status from 3 of 5 devices you can see; 1 hasn't checked in yet and your access doesn't cover Support Portal on 1 more.",
		});
	});

	test("crashing outranks a safe update in the same app", () => {
		const input = onStudio(
			placement({
				id: "invoice-studio",
				project_id: APP.invoiceAi,
				desired_state: "running",
				observed_state: "backoff",
			}),
		);
		expect(appHeadline(input, app(APP.invoiceAi, "Invoice AI"))).toMatchObject({
			code: "app.crashing",
			params: { service: "invoice-studio", device: "studio-mac-mini" },
		});
	});

	test("buffered writes need you", () => {
		expect(appText(sampleFleet(), app(APP.fieldNotes, "Field Notes"))).toEqual({
			lead: "Buffered changes from field-notes on studio-mac-mini need you.",
			rest: "1 change conflicts with newer cloud data and 3 are paused because cloud access changed.",
		});
	});

	test("updating with a deadline", () => {
		expect(appText(sampleFleet(), app(APP.invoiceAi, "Invoice AI"))).toEqual({
			lead: "invoice-extractor on edge-berlin-01 is switching to settings v12.",
			rest: "If it isn't healthy by 14:01, the device restores settings v11 on its own. Status from 3 of 5 devices you can see; 1 is unknown, 1 hasn't checked in yet.",
		});
	});

	test("an update that is still being checked names the version it switches to", () => {
		const input = sampleFleet();
		const rollouts = input.live[ID.edge].rollouts;
		if (!rollouts) throw new Error("fixture: rollouts");
		rollouts[0] = {
			...rollouts[0],
			state: "validating",
			active_revision: null,
		};
		expect(appText(input, app(APP.invoiceAi, "Invoice AI")).lead).toBe(
			"invoice-extractor on edge-berlin-01 is switching to settings v12.",
		);
	});

	test("staged: the device discards it a day after it was staged", () => {
		const input = sampleFleet();
		const rollouts = input.live[ID.edge].rollouts;
		if (!rollouts) throw new Error("fixture: rollouts");
		const stagedAt = SAMPLE_NOW - 3_600;
		rollouts[0] = {
			...rollouts[0],
			state: "staged",
			active_revision: null,
			created_at: stagedAt,
			updated_at: stagedAt,
			deadline_at: null,
		};
		expect(appText(input, app(APP.invoiceAi, "Invoice AI"))).toEqual({
			lead: "An update for invoice-extractor on edge-berlin-01 is ready but not active.",
			rest: `Activate it to switch to settings v12; it's discarded on ${time.at(stagedAt + 86_400)} otherwise.`,
		});
	});

	test("applying and unknown state", () => {
		const applying = withLivePlacement(ID.edge, "support-bot", {
			config_revision: 8,
		});
		expect(
			appText(applying, app(APP.supportPortal, "Support Portal")).lead,
		).toBe("support-bot on edge-berlin-01 is applying changes.");
		const unknown = withLivePlacement(ID.edge, "support-bot", {
			observed_state: "paused_by_host",
		});
		expect(
			appText(unknown, app(APP.supportPortal, "Support Portal")).lead,
		).toBe("support-bot on edge-berlin-01 is in an unknown state.");
	});

	test("stopped as asked, events nowhere and a paused upload", () => {
		const crm = app(APP.crmSync, "CRM Sync", {
			events: { total: 2, eligible: 2, nowhere: ["Weekly export"] },
		});
		expect(appText(sampleFleet(), crm)).toEqual({
			lead: "nightly-sync on edge-berlin-01 is stopped, as you asked.",
			rest: "Weekly export isn't on any device. An upload to edge-berlin-01 is paused at 12 of 38 files.",
		});
		const two = {
			...crm,
			events: { ...crm.events, nowhere: ["Weekly export", "Lead import"] },
		};
		expect(appText(sampleFleet(), two).rest).toStartWith(
			"Weekly export and Lead import aren't on any device.",
		);
	});

	test("all stopped, as asked", () => {
		expect(appText(sampleFleet(), app(APP.crmSync, "CRM Sync")).lead).toBe(
			"CRM Sync is stopped on 1 device, as you asked.",
		);
	});

	test("all as asked, with drift and coverage", () => {
		const portal = app(APP.supportPortal, "Support Portal", {
			latestLabel: "v2.4.0",
			olderServices: 1,
		});
		expect(appText(sampleFleet(), portal)).toEqual({
			lead: "Support Portal runs as you asked on 1 device.",
			rest: "1 service runs an older version than v2.4.0. Status from 3 of 5 devices you can see; 1 hasn't checked in yet and your access doesn't cover Support Portal on 1 more.",
		});
		const two = onStudio(
			placement({
				id: "support-bot",
				project_id: APP.supportPortal,
				desired_state: "running",
				observed_state: "running",
			}),
		);
		expect(appText(two, app(APP.supportPortal, "Support Portal")).lead).toBe(
			"Support Portal runs as you asked on 2 devices.",
		);
	});

	test("the one coverage sentence in every combination (APP §6.3)", () => {
		const text = (params: Record<string, number>) =>
			headlinePartCopy(
				t,
				{
					code: "app.coverage",
					params: {
						app: "CRM Sync",
						readable: 3,
						total: 9,
						unknown: 0,
						never: 0,
						noAccess: 0,
						...params,
					},
				},
				time,
			);
		const status = "Status from 3 of 9 devices you can see";
		expect(text({})).toBe(`${status}.`);
		expect(text({ unknown: 2 })).toBe(`${status}; 2 are unknown.`);
		expect(text({ never: 2 })).toBe(`${status}; 2 haven't checked in yet.`);
		expect(text({ noAccess: 2 })).toBe(
			`${status}; your access doesn't cover CRM Sync on 2 more.`,
		);
		expect(text({ unknown: 1, noAccess: 1 })).toBe(
			`${status}; 1 is unknown and your access doesn't cover CRM Sync on 1 more.`,
		);
		expect(text({ unknown: 2, never: 1, noAccess: 3 })).toBe(
			`${status}; 2 are unknown, 1 hasn't checked in yet and your access doesn't cover CRM Sync on 3 more.`,
		);
	});
});
