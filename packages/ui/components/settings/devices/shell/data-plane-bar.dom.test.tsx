import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, act } from "react";
import { ApiResponseError } from "../../../../lib/api-error";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
	settle,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type { PlaneLine, PlaneSegmentId } from "./plane-popover";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const area = await import("../primitives/area-context");
const { DataPlaneBar, DataPlaneBarView, fitPlaneBar } = await import(
	"./data-plane-bar"
);
const { useOverlayStore } = await import("../workspace");

const ACCOUNT: DevicesScope = { kind: "account" };

async function mountBar(options: MountDevicesOptions = {}) {
	const navigations: DevicesRoute[] = [];
	const navigate = (route: DevicesRoute) => {
		navigations.push(route);
	};
	const bar = <DataPlaneBar scope={ACCOUNT} onNavigate={navigate} />;
	const mounted = await mountDevices(bar, options);
	return { mounted, navigations };
}

const noPlane = () => null;
const dotOf = (el: Element) => el.getAttribute("data-dot");
const firstCell = (row: readonly string[]) => row[0];

function stateOf(rows: readonly string[][], name: string) {
	for (const row of rows) {
		if (row[0] === name) return row[1];
	}
	return undefined;
}

const segmentTexts = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-plane]"), (el) => {
		return (el.textContent ?? "").replace(/\s+/g, " ").trim();
	});

const tableRows = (root: ParentNode) =>
	Array.from(root.querySelectorAll("tbody tr"), (row) => {
		return Array.from(row.querySelectorAll("td"), (cell) => {
			return (cell.textContent ?? "").trim();
		});
	});

const NOW_MS = 1_790_769_612_000;

const LINES: PlaneLine[] = [
	{
		id: "hub",
		state: "ok",
		word: "Current",
		short: "checked 13 s ago",
		text: "checked 13 s ago",
	},
	{
		id: "status",
		state: "ok",
		word: "Readable",
		short: "3 of 5 readable",
		text: "3 of 5 readable · newest 41 s ago",
	},
	{
		id: "live",
		state: "ok",
		word: "Connected",
		short: "2 connections",
		text: "2 connections · refreshed every 15 s",
	},
	{
		id: "local",
		state: "ok",
		word: "Kept safely",
		short: "keys for 6 devices",
		text: "Desktop app · keys for 6 devices · kept safely",
	},
	{
		id: "device",
		state: "off",
		word: "Needs a shell",
		short: "2 to check",
		text: "2 devices need a local check",
	},
	{
		id: "certificates",
		state: "ok",
		word: "Reported",
		short: "3 of 5 reported",
		text: "3 devices reported",
	},
];

const FAILING: PlaneLine[] = [
	{
		id: "hub",
		state: "err",
		word: "Couldn't refresh",
		short: "couldn't refresh since 13:59:58",
		text: "couldn't refresh · data from 13:59:58 · retry in 0:27",
	},
	...LINES.slice(1),
];

function At({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<area.AreaNowContext.Provider value={NOW_MS}>
			{children}
		</area.AreaNowContext.Provider>
	);
}

const planeIds = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-plane]"), (el) => {
		return el.getAttribute("data-plane");
	});

const SEGMENT_PX = { name: 60, icon: 19, tight: 6, text: 100 };
const CLOCK_PX = 120;
const WORST_PX = 200;

/** happy-dom lays nothing out: the widths the bar's CSS gives its children. */
function layOut(bar: HTMLElement, width: number) {
	const on = (flag: string) => bar.hasAttribute(`data-fit-${flag}`);
	const segmentWidth = (segment: Element) =>
		SEGMENT_PX.name +
		(on("icons") ? 0 : SEGMENT_PX.icon) -
		(on("tight") ? SEGMENT_PX.tight : 0) +
		(segment.hasAttribute("data-terse") ? 0 : SEGMENT_PX.text);
	const widthOf = (child: Element) => {
		if (child.hasAttribute("data-plane-worst")) {
			return on("worst") ? WORST_PX : 0;
		}
		if (child.hasAttribute("data-plane")) {
			return on("worst") ? 0 : segmentWidth(child);
		}
		return on("clock") ? 0 : CLOCK_PX;
	};
	Object.defineProperty(bar, "clientWidth", {
		configurable: true,
		value: width,
	});
	for (const child of Array.from(bar.children)) {
		Object.defineProperty(child, "offsetWidth", {
			configurable: true,
			get: () => widthOf(child),
		});
	}
}

const fitFlags = (bar: HTMLElement) =>
	bar
		.getAttributeNames()
		.filter((name) => name.startsWith("data-fit-"))
		.map((name) => name.slice("data-fit-".length))
		.sort();

const terseIds = (bar: HTMLElement) =>
	Array.from(bar.querySelectorAll("[data-plane][data-terse]"), (el) => {
		return el.getAttribute("data-plane");
	});

async function renderBar(segments: PlaneLine[]) {
	const opened: (PlaneSegmentId | undefined)[] = [];
	const ui = () => (
		<At>
			<DataPlaneBarView
				segments={segments}
				renderPlane={noPlane}
				onOpenAll={(plane) => {
					opened.push(plane);
				}}
			/>
		</At>
	);
	const rendered = await dom.render(ui());
	const bar = rendered.container.querySelector("footer") as HTMLElement;
	return { bar, opened, refit: () => rendered.rerender(ui()) };
}

async function renderCompact(segments: PlaneLine[]) {
	const opened: (PlaneSegmentId | undefined)[] = [];
	const openAll = (plane: PlaneSegmentId | undefined) => {
		opened.push(plane);
	};
	const { container } = await dom.render(
		<At>
			<DataPlaneBarView
				compact={true}
				segments={segments}
				renderPlane={noPlane}
				onOpenAll={openAll}
			/>
		</At>,
	);
	return { container, opened };
}

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

describe("data-plane bar", () => {
	test("six segments in order, each with a state shape, a name and its text, plus the zone", async () => {
		const { container } = await dom.render(
			<At>
				<DataPlaneBarView segments={LINES} renderPlane={noPlane} />
			</At>,
		);
		const bar = container.querySelector("footer") as HTMLElement;
		expect(bar.getAttribute("aria-label")).toBe("Data sources and freshness");
		expect(bar.className).toContain("h-7");
		expect(container.innerHTML).not.toContain("fixed");
		expect(planeIds(bar)).toEqual([
			"hub",
			"status",
			"live",
			"local",
			"device",
			"certificates",
		]);
		expect(segmentTexts(bar)).toEqual([
			"Hubchecked 13 s ago",
			"Encrypted status3 of 5 readable",
			"Live2 connections",
			"This computerkeys for 6 devices",
			"On-device only2 to check",
			"Certificates3 of 5 reported",
		]);
		const dots = Array.from(bar.querySelectorAll("[data-dot]"), dotOf);
		expect(dots).toEqual(["ok", "ok", "ok", "ok", "off", "ok"]);
		const clock = bar.querySelector("[data-plane-clock]") as HTMLElement;
		expect(clock.querySelector("time")?.textContent?.length).toBeGreaterThan(4);
		expect(clock.textContent?.length).toBeGreaterThan(
			clock.querySelector("time")?.textContent?.length ?? 0,
		);
		expect(clock.getAttribute("title")).toContain("time zone");
	});

	test("a segment opens its plane's popover", async () => {
		const rendered: PlaneSegmentId[] = [];
		const renderPlane = (plane: PlaneSegmentId) => {
			rendered.push(plane);
			return <p>about {plane}</p>;
		};
		await dom.render(
			<At>
				<DataPlaneBarView segments={LINES} renderPlane={renderPlane} />
			</At>,
		);
		expect(queryByRole("dialog")).toBeNull();
		expect(rendered).toEqual([]);
		const segment = byRole("button", /Encrypted status/);
		expect(segment.getAttribute("title")).toBe(
			"Encrypted status · Readable · refreshed on change, at least every 60 s. Select for details.",
		);
		await click(segment);
		await settle();
		expect(byRole("dialog", "Encrypted status data source").textContent).toBe(
			"about status",
		);
		expect(new Set(rendered)).toEqual(new Set(["status"]));
	});

	test("a failing plane colours its text and keeps its shape", async () => {
		const { container } = await dom.render(
			<At>
				<DataPlaneBarView segments={FAILING} renderPlane={noPlane} />
			</At>,
		);
		const hub = container.querySelector("[data-plane=hub]") as HTMLElement;
		expect(hub.getAttribute("data-plane-state")).toBe("err");
		expect(hub.querySelector("[data-dot=err]")).not.toBeNull();
		expect(hub.textContent).toContain("couldn't refresh since 13:59:58");
		expect(hub.querySelector(".text-critical")).not.toBeNull();
	});

	test("segments keep their content width: nothing in them can ellipsize", async () => {
		const { bar } = await renderBar(LINES);
		const segments = Array.from(bar.querySelectorAll("[data-plane]"));
		expect(segments).toHaveLength(6);
		for (const segment of segments) {
			expect(segment.className).toContain("shrink-0");
			expect(segment.className).not.toContain("min-w-0");
			expect(segment.querySelector(".truncate, .min-w-0")).toBeNull();
		}
		expect(fitFlags(bar)).toEqual([]);
		expect(terseIds(bar)).toEqual([]);
	});

	test("too narrow: icons, then padding, then the text of calm planes in a fixed order", async () => {
		const { bar } = await renderBar(LINES);
		layOut(bar, 1214);
		expect(fitPlaneBar(bar)).toBe(0);
		expect(fitFlags(bar)).toEqual([]);

		layOut(bar, 1100);
		expect(fitPlaneBar(bar)).toBe(1);
		expect(fitFlags(bar)).toEqual(["icons"]);
		expect(terseIds(bar)).toEqual([]);

		layOut(bar, 1000);
		expect(fitPlaneBar(bar)).toBe(3);
		expect(fitFlags(bar)).toEqual(["icons", "tight"]);
		expect(terseIds(bar)).toEqual(["live"]);

		layOut(bar, 800);
		fitPlaneBar(bar);
		expect(terseIds(bar)).toEqual(["live", "device", "certificates"]);

		layOut(bar, 700);
		fitPlaneBar(bar);
		expect(terseIds(bar)).toEqual(["status", "live", "device", "certificates"]);
		expect(fitFlags(bar)).toEqual(["icons", "tight"]);
		const live = bar.querySelector("[data-plane=live]") as HTMLElement;
		expect(live.textContent).toBe("Live2 connections");
		expect(live.querySelector("span:last-child")?.className).toContain(
			"group-data-[terse]/plane:sr-only",
		);
	});

	test("a plane that needs attention keeps its text; the clock goes before it does", async () => {
		const { bar } = await renderBar(FAILING);
		layOut(bar, 480);
		expect(fitPlaneBar(bar)).toBe(9);
		expect(fitFlags(bar)).toEqual(["clock", "icons", "tight"]);
		expect(terseIds(bar)).toEqual([
			"status",
			"live",
			"local",
			"device",
			"certificates",
		]);
	});

	test("a fuller form comes back only with room to spare, so a ticking age can't flip it", async () => {
		const { bar } = await renderBar(LINES);
		layOut(bar, 1100);
		const level = fitPlaneBar(bar);
		expect(level).toBe(1);
		layOut(bar, 1220);
		expect(fitPlaneBar(bar, level)).toBe(1);
		expect(fitFlags(bar)).toEqual(["icons"]);
		layOut(bar, 1254);
		expect(fitPlaneBar(bar, level)).toBe(0);
		expect(fitFlags(bar)).toEqual([]);
	});

	test("nothing fits: the one Data sources button stands in, and leaves again when there is room", async () => {
		const { bar, opened, refit } = await renderBar(FAILING);
		expect(queryByRole("button", /Data sources/)).toBeNull();
		layOut(bar, 300);
		await refit();
		expect(fitFlags(bar)).toEqual(["clock", "icons", "tight", "worst"]);
		expect(planeIds(bar)).toHaveLength(6);
		const button = byRole("button", /Data sources/);
		expect(button.className).toContain(
			"hidden group-data-[fit-worst]/planes:inline-flex",
		);
		expect(button.textContent).toContain(
			"Hub: couldn't refresh since 13:59:58",
		);
		await click(button);
		expect(opened).toEqual(["hub"]);

		layOut(bar, 1300);
		await refit();
		expect(fitFlags(bar)).toEqual([]);
		expect(queryByRole("button", /Data sources/)).toBeNull();
	});

	test("phone: one button names the worst plane and opens the list of all six", async () => {
		const { container, opened } = await renderCompact(FAILING);
		expect(planeIds(container)).toEqual([]);
		const button = byRole("button", /Data sources/);
		expect(button.textContent).toContain(
			"Hub: couldn't refresh since 13:59:58",
		);
		expect(button.querySelector("[data-dot=err]")).not.toBeNull();
		await click(button);
		expect(opened).toEqual(["hub"]);
		expect(container.querySelector("[data-plane-clock]")).not.toBeNull();
	});

	test("phone, nothing wrong: all current", async () => {
		const { opened } = await renderCompact(LINES);
		const button = byRole("button", /Data sources/);
		expect(button.textContent).toContain("all current");
		expect(button.querySelector("[data-dot=ok]")).not.toBeNull();
		await click(button);
		expect(opened).toEqual([undefined]);
	});
});

const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\b[GD]\d{1,2}\b/;

describe("data-plane bar over the workspace", () => {
	test("states what each plane knows about the fleet right now", async () => {
		const { mounted } = await mountBar();
		expect(segmentTexts(mounted.container)).toEqual([
			"Hubchecked now",
			"Encrypted status2 of 5 readable",
			"Live2 connections",
			"This computerkeys for 7 devices",
			"On-device only2 to check",
			"Certificates2 of 5 reported",
		]);
		expect(mounted.container.textContent).not.toMatch(MACHINE_WORDS);
		expect(mounted.container.innerHTML).not.toContain("fixed");
	});

	test("a plane's popover explains it and lists every device's state on it", async () => {
		const { mounted, navigations } = await mountBar();
		await click(byRole("button", /^Encrypted status/));
		await mounted.settle();
		const popover = byRole("dialog", "Encrypted status data source");
		expect(popover.textContent).toContain("Only keys on this computer");
		expect(popover.textContent).toContain("2 of 5 readable");
		const rows = tableRows(popover);
		expect(rows).toHaveLength(7);
		expect(stateOf(rows, "lab-gpu-02")).toBe("Locked");
		expect(stateOf(rows, "old-kiosk")).toBe("Not read");
		expect(popover.textContent).not.toMatch(MACHINE_WORDS);

		await click(byRole("button", "Unlock several…"));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock_several");
		expect(queryByRole("dialog")).toBeNull();

		await click(byRole("button", /^Encrypted status/));
		await mounted.settle();
		await click(byRole("link", "warehouse-pi"));
		expect(navigations).toEqual([
			{ screen: "device", deviceId: SAMPLE_IDS.warehouse, tab: "overview" },
		]);
	});

	test("On-device only lists what needs a shell and offers Diagnose", async () => {
		const { mounted } = await mountBar();
		await click(byRole("button", /^On-device only/));
		await mounted.settle();
		const popover = byRole("dialog", "On-device only data source");
		const rows = tableRows(popover);
		expect(rows.map(firstCell).sort()).toEqual([
			"cold-storage-nas",
			"warehouse-pi",
		]);
		await click(allByRole("button", "Diagnose", popover)[0]);
		expect(useOverlayStore.getState().overlay.kind).toBe("diagnose");
	});

	test("a hub that stops answering keeps its data, says since when, and offers Retry now", async () => {
		const { mounted } = await mountBar();
		const stop = mounted.fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({ status: 403, code: "FORBIDDEN", message: "no" }),
		);
		await act(async () => {
			await mounted.fake.queryClient.refetchQueries();
		});
		await mounted.settle();
		const hub = mounted.container.querySelector(
			"[data-plane=hub]",
		) as HTMLElement;
		expect(hub.getAttribute("data-plane-state")).toBe("err");
		expect(hub.textContent).toContain("couldn't refresh");
		expect(segmentTexts(mounted.container)[1]).toBe(
			"Encrypted status2 of 5 readable",
		);

		await click(hub);
		await mounted.settle();
		const popover = byRole("dialog", "Hub data source");
		expect(popover.textContent).toContain("Couldn't refresh");
		expect(tableRows(popover)).toHaveLength(7);
		stop();
		await click(byRole("button", "Retry now"));
		await mounted.settle();
		expect(
			mounted.container
				.querySelector("[data-plane=hub]")
				?.getAttribute("data-plane-state"),
		).toBe("ok");
	});

	test("phone: one button, and tapping it asks for the sheet of all planes", async () => {
		const { mounted } = await mountBar({ widthBucket: "phone" });
		expect(segmentTexts(mounted.container)).toEqual([]);
		const button = byRole("button", /Data sources/);
		expect(button.textContent).toContain("all current");
		await click(button);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "plane",
			plane: "hub",
		});
	});

	test("under 900 px the six segments give way to the one button too", async () => {
		const { mounted } = await mountBar({ widthBucket: "narrow" });
		expect(segmentTexts(mounted.container)).toEqual([]);
		expect(byRole("button", /Data sources/).textContent).toContain(
			"all current",
		);
		const bar = mounted.container.querySelector("footer") as HTMLElement;
		expect(bar.hasAttribute("data-compact")).toBe(true);
	});

	test("browser keys the browser may delete show as a warning on This computer", async () => {
		const { mounted } = await mountBar({
			platform: "web",
			persistence: "denied",
		});
		const local = mounted.container.querySelector(
			"[data-plane=local]",
		) as HTMLElement;
		expect(local.getAttribute("data-plane-state")).toBe("warn");
		expect(local.textContent).toContain("keys may be deleted");
	});

	test("older hub: every plane still reports; nothing reads as an error", async () => {
		const { mounted } = await mountBar({ hubVersion: "old" });
		const texts = segmentTexts(mounted.container);
		expect(texts).toHaveLength(6);
		expect(texts[0]).toBe("Hubchecked now");
		expect(texts[1]).toBe("Encrypted status2 of 5 readable");
		expect(texts[5]).toMatch(/^Certificates\d of 5 reported$/);
		expect(mounted.container.querySelector("[data-dot=err]")).toBeNull();
		expect(mounted.container.textContent).not.toMatch(MACHINE_WORDS);
	});
});
