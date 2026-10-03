import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import type { DevicesScope } from "../../../../lib/device-management/model/types";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import { byRole, byText, click, installDom } from "../testing/dom-harness";
import type { PlaneDeviceRow, PlaneFacts, PlaneLine } from "./plane-popover";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const area = await import("../primitives/area-context");
const planes = await import("./plane-popover");
const {
	PLANE_SEGMENTS,
	PlaneDetail,
	planeCadence,
	planeExplanation,
	planeName,
	planeReaders,
	planeSegments,
	worstPlane,
} = planes;

const NOW_S = 1_790_769_600;
const t = getI18n().getFixedT("en", "devices") as DevicesT;

const TIME: AreaTime = {
	now: NOW_S * 1000,
	nowS: NOW_S,
	locale: "en",
	ago: (atS) => `${Math.round(NOW_S - atS)} s ago`,
	clock: (atS) => `clock:${atS - NOW_S}`,
	abs: (atS) => `abs:${atS}`,
	at: (atS) => `at:${atS}`,
	countdown: (untilS) => `0:${Math.round(untilS - NOW_S)}`,
};

const FACTS: PlaneFacts = {
	hub: { state: "on", failing: false, checkedAt: NOW_S - 13 },
	status: {
		active: 5,
		readable: 3,
		locked: 2,
		newestAt: NOW_S - 41,
		failing: 0,
	},
	live: { connections: 2, reconnecting: 0 },
	local: { platform: "desktop", persisted: true, keys: 6 },
	device: { toCheck: 2 },
	certificates: { total: 5, reported: 3, never: 1, noAccess: 1 },
};

const ACCOUNT: DevicesScope = { kind: "account" };

function line(facts: PlaneFacts, id: PlaneLine["id"]) {
	for (const entry of planeSegments(t, TIME, facts)) {
		if (entry.id === id) return entry;
	}
	throw new Error(`No ${id} line`);
}

const idOf = (entry: PlaneLine) => entry.id;
const nameOf = (entry: PlaneLine) => planeName(t, entry.id);
const shortOf = (entry: PlaneLine) => entry.short;
const stateOf = (entry: PlaneLine) => entry.state;

const cellText = (cell: Element) => cell.textContent;
const rowCells = (row: Element) =>
	Array.from(row.querySelectorAll("td"), cellText);
const tableCells = (root: ParentNode) =>
	Array.from(root.querySelectorAll("tbody tr"), rowCells);

interface AtProps {
	children: ReactNode;
}

function At({ children }: Readonly<AtProps>) {
	return (
		<area.AreaNowContext.Provider value={NOW_S * 1000}>
			{children}
		</area.AreaNowContext.Provider>
	);
}

const DETAIL_ROWS: PlaneDeviceRow[] = [
	{
		deviceId: "d1",
		name: "edge-berlin-01",
		state: "Current",
		at: NOW_S - 41,
	},
	{ deviceId: "d2", name: "lab-gpu-02", state: "Locked" },
	{
		deviceId: "d3",
		name: "warehouse-pi",
		state: "Offline",
		action: <button type="button">Diagnose</button>,
	},
];

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("plane lines", () => {
	test("six planes in status-bar order, each with a state word and a sentence", () => {
		const lines = planeSegments(t, TIME, FACTS);
		expect(lines.map(idOf)).toEqual([...PLANE_SEGMENTS]);
		expect(lines.map(nameOf)).toEqual([
			"Hub",
			"Encrypted status",
			"Live",
			"This computer",
			"On-device only",
			"Certificates",
		]);
		expect(lines.map(shortOf)).toEqual([
			"checked 13 s ago",
			"3 of 5 readable",
			"2 connections",
			"keys for 6 devices",
			"2 to check",
			"3 of 5 reported",
		]);
		expect(lines.map(stateOf)).toEqual(["ok", "ok", "ok", "ok", "off", "ok"]);
		expect(line(FACTS, "status").text).toBe(
			"3 of 5 readable · newest 41 s ago · 2 locked",
		);
		expect(line(FACTS, "live").text).toBe(
			"2 connections · refreshed every 15 s",
		);
		expect(line(FACTS, "local").text).toBe(
			"Desktop app · keys for 6 devices · kept safely",
		);
		expect(line(FACTS, "device").text).toBe("2 devices need a local check");
		expect(line(FACTS, "certificates").text).toBe(
			"3 devices reported · 1 not yet · 1 no access",
		);
		expect(worstPlane(lines)).toBeUndefined();
	});

	test("a failing hub keeps its data and says since when and when it retries", () => {
		const facts: PlaneFacts = {
			...FACTS,
			hub: {
				state: "on",
				failing: true,
				dataFrom: NOW_S - 2,
				retryAt: NOW_S + 27,
			},
		};
		const hub = line(facts, "hub");
		expect(hub.state).toBe("err");
		expect(hub.word).toBe("Couldn't refresh");
		expect(hub.text).toBe(
			"couldn't refresh · data from clock:-2 · retry in 0:27",
		);
		expect(hub.short).toBe("couldn't refresh since clock:-2");
		expect(worstPlane(planeSegments(t, TIME, facts))?.id).toBe("hub");
	});

	test("hub checking, unreachable and off are states of their own", () => {
		const checking = line(
			{
				...FACTS,
				hub: { state: "checking", failing: false, host: "hub.example.com" },
			},
			"hub",
		);
		expect(checking.state).toBe("busy");
		expect(checking.text).toBe("checking hub.example.com…");
		const unreachable = line(
			{ ...FACTS, hub: { state: "unreachable", failing: false } },
			"hub",
		);
		expect(unreachable.state).toBe("err");
		expect(unreachable.word).toBe("Unreachable");
		const off = line({ ...FACTS, hub: { state: "off", failing: true } }, "hub");
		expect(off.text).toBe("devices are off on this hub");
	});

	test("nothing readable, nothing connected, nothing to check, nothing reported", () => {
		const facts: PlaneFacts = {
			...FACTS,
			status: { active: 4, readable: 0, locked: 4, failing: 0 },
			live: { connections: 0, reconnecting: 0 },
			device: { toCheck: 0 },
			certificates: { total: 4, reported: 0, never: 4, noAccess: 0 },
		};
		expect(line(facts, "status")).toMatchObject({
			state: "off",
			word: "Nothing readable",
			text: "0 of 4 readable · 4 locked",
		});
		expect(line(facts, "live")).toMatchObject({
			state: "off",
			short: "not connected",
		});
		expect(line(facts, "device").short).toBe("nothing to check");
		expect(line(facts, "certificates")).toMatchObject({
			state: "off",
			text: "0 devices reported · 4 not yet",
		});
	});

	test("warnings: a snapshot that couldn't refresh, a reconnecting session, browser keys at risk", () => {
		const facts: PlaneFacts = {
			...FACTS,
			status: { ...FACTS.status, failing: 1 },
			live: { connections: 1, reconnecting: 1 },
			local: { platform: "web", persisted: false, keys: 1 },
		};
		expect(line(facts, "status").state).toBe("warn");
		expect(line(facts, "status").text).toContain("1 couldn't refresh");
		expect(line(facts, "live")).toMatchObject({
			state: "warn",
			short: "1 connection · 1 reconnecting",
		});
		expect(line(facts, "local")).toMatchObject({
			state: "warn",
			word: "Keys at risk",
			short: "keys may be deleted",
			text: "Web · keys for 1 device · the browser may delete them",
		});
		expect(worstPlane(planeSegments(t, TIME, facts))?.id).toBe("status");
	});

	test("every plane explains itself, its cadence and who can read it", () => {
		for (const id of PLANE_SEGMENTS) {
			expect(planeExplanation(t, id).length).toBeGreaterThan(30);
			expect(planeCadence(t, id).length).toBeGreaterThan(4);
			expect(planeReaders(t, id).length).toBeGreaterThan(10);
		}
		expect(planeExplanation(t, "certificates")).toContain("at least hourly");
	});
});

describe("plane detail", () => {
	test("states the plane, its line, what it is, cadence, readers and each device", async () => {
		const opened: string[] = [];
		const open = (id: string) => {
			opened.push(id);
		};
		const actions = <button type="button">Unlock several…</button>;
		const status = line(FACTS, "status");
		const { container } = await dom.render(
			<At>
				<PlaneDetail
					segment={status}
					scope={ACCOUNT}
					rows={DETAIL_ROWS}
					actions={actions}
					onOpenDevice={open}
				/>
			</At>,
		);
		const detail = container.querySelector("[data-chrome=plane-detail]");
		expect(detail?.getAttribute("data-plane")).toBe("status");
		expect(byRole("heading", /Encrypted status/).textContent).toContain(
			"Readable",
		);
		expect(container.textContent).toContain(
			"3 of 5 readable · newest 41 s ago · 2 locked",
		);
		expect(container.textContent).toContain(
			"Only keys on this computer can read it",
		);
		expect(container.textContent).toContain(
			"Refreshed on change, at least every 60 s · Who can read it: Only this computer, with the device's keys unlocked.",
		);
		expect(byRole("button", "Unlock several…")).toBeTruthy();
		const cells = tableCells(container);
		expect(cells[0]?.[0]).toBe("edge-berlin-01");
		expect(cells[0]?.[1]).toBe("Current");
		expect(cells[0]?.[2]).not.toBe("–");
		expect(cells[1]).toEqual(["lab-gpu-02", "Locked", "–"]);
		expect(cells[2]?.[2]).toBe("Diagnose");
		const link = byText("lab-gpu-02");
		expect(link.getAttribute("href")).toContain("device=d2");
		await click(link);
		expect(opened).toEqual(["d2"]);
	});

	test("a plane without devices to list says so", async () => {
		const device = line(FACTS, "device");
		const { container } = await dom.render(
			<At>
				<PlaneDetail segment={device} scope={ACCOUNT} rows={[]} />
			</At>,
		);
		expect(container.querySelector("tbody")?.textContent).toBe(
			"No devices to show here.",
		);
		expect(container.querySelector("[data-dot=off]")).not.toBeNull();
	});
});
