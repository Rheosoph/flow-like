import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	test,
} from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	allByRole,
	byRole,
	byText,
	click,
	clickByText,
	fire,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type {
	DeviceRailViewProps,
	RailChip,
	RailDevice,
	RailGroupId,
	RailItem,
	RailPackage,
} from "./device-rail";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
await preloadDevices();
const rail = await import("./device-rail");
const { DeviceRail, DeviceRailView, groupRailDevices, matchesRail, railItems } =
	rail;
const { useOverlayStore } = await import("../workspace");

const ACCOUNT: DevicesScope = { kind: "account" };
const FLEET: DevicesRoute = { screen: "fleet", view: "devices" };

function device(id: string, over: Partial<RailDevice> = {}): RailDevice {
	return {
		id,
		name: id,
		presence: "online",
		relationship: "owner",
		health: "healthy",
		counts: {},
		keyState: "unlocked",
		...over,
	};
}

const SAMPLE: RailDevice[] = [
	device("edge-berlin-01", {
		health: "attention",
		counts: { warning: 2, notice: 1 },
		keyState: "live",
		sub: "Certificate expiring · online · 0.9.4",
		services: "invoice-extractor support-bot Invoice AI",
	}),
	device("warehouse-pi", {
		presence: "offline",
		presenceLabel: "Offline since 11:00",
		health: "critical",
		counts: { critical: 2, warning: 1, notice: 1 },
		sub: "Crashing · offline · 0.9.2",
		services: "scanner-ingest Warehouse Scanner",
	}),
	device("studio-mac-mini", {
		health: "attention",
		counts: { warning: 2, notice: 2 },
		keyState: "live",
	}),
	device("cold-storage-nas", {
		presence: "never",
		health: "attention",
		counts: { warning: 2 },
		keyState: "locked",
	}),
	device("lab-gpu-02", {
		relationship: "shared",
		health: "unknown",
		keyState: "locked",
		sub: "Mira Novak · locked",
	}),
	device("old-kiosk", {
		presence: "revoked",
		health: "revoked",
		keyState: "stale",
	}),
	device("partner-edge", {
		presence: "revoked",
		relationship: "shared",
		health: "revoked",
		keyState: "none",
	}),
];

const PACKAGES: RailPackage[] = [
	{
		id: "enr-1",
		name: "factory-line-3",
		sub: "Waiting · expires in 20 h",
		expired: false,
	},
	{
		id: "enr-2",
		name: "test-vm",
		sub: "Expired 29 Sept · unusable",
		expired: true,
	},
];

interface Nav {
	route: DevicesRoute;
	replace: boolean;
}

function recorder() {
	const navigations: Nav[] = [];
	const navigate: DeviceRailViewProps["onNavigate"] = (route, options) => {
		navigations.push({ route, replace: options?.replace ?? false });
	};
	return { navigations, navigate };
}

function view(over: Partial<DeviceRailViewProps> = {}) {
	const { navigations, navigate } = recorder();
	const node = (
		<DeviceRailView
			scope={ACCOUNT}
			route={FLEET}
			onNavigate={navigate}
			mode="docked"
			devices={SAMPLE}
			packages={PACKAGES}
			attentionTotal={15}
			unlockedCount={3}
			{...over}
		/>
	);
	return { node, navigations };
}

const groupIds = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-rail-group]"), (el) => {
		return el.getAttribute("data-rail-group");
	});

const rowNames = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-rail-row]"), (el) => {
		return el.querySelector("[title]")?.getAttribute("title");
	});

const textOf = (root: ParentNode, selector: string) =>
	root.querySelector(selector)?.textContent;

interface WithState {
	state: string;
}

const idOf = (row: RailDevice) => row.id;
const routeOf = (nav: Nav) => nav.route;
const contentOf = (el: Element) => el.textContent ?? "";
const stateOf = (session: WithState) => session.state;

function matching(chip: RailChip, filter?: Parameters<typeof matchesRail>[2]) {
	const ids: string[] = [];
	for (const row of SAMPLE) {
		if (matchesRail(row, chip, filter)) ids.push(row.id);
	}
	return ids;
}

function headsOf(items: readonly RailItem[]) {
	return items.flatMap((item) => {
		return item.kind === "head" ? [item] : [];
	});
}

function headOf(items: readonly RailItem[], group: RailGroupId) {
	for (const item of headsOf(items)) {
		if (item.group === group) return item;
	}
	return undefined;
}

const groupOfHead = (head: ReturnType<typeof headsOf>[number]) => head.group;

function moreOf(items: readonly RailItem[]) {
	return items.flatMap((item) => {
		return item.kind === "more" ? [item] : [];
	});
}

const hiddenOf = (more: ReturnType<typeof moreOf>[number]) => more.hidden;

function noteGroups(items: readonly RailItem[]) {
	return items.flatMap((item) => {
		return item.kind === "note" ? [item.group] : [];
	});
}

function countOf(items: readonly RailItem[], kind: RailItem["kind"]) {
	let count = 0;
	for (const item of items) {
		if (item.kind === kind) count += 1;
	}
	return count;
}

function warned(count: number, pad: number, prefix = "warn") {
	return Array.from({ length: count }, (_, index) => {
		return device(`${prefix}-${String(index).padStart(pad, "0")}`, {
			health: "attention",
			counts: { warning: 1 },
		});
	});
}

const LAYOUT_KEYS = ["offsetHeight", "offsetWidth"] as const;

/** happy-dom has no layout: the rail body gets a viewport, every measured item one row's height. Returns the undo. */
function fakeViewport(height: number, width: number) {
	const proto = window.HTMLElement.prototype;
	const saved = LAYOUT_KEYS.map((key) => {
		return [key, Object.getOwnPropertyDescriptor(proto, key)] as const;
	});
	Object.defineProperty(proto, "offsetHeight", {
		configurable: true,
		get(this: HTMLElement) {
			return this.hasAttribute("data-rail-body") ? height : 44;
		},
	});
	Object.defineProperty(proto, "offsetWidth", {
		configurable: true,
		value: width,
	});
	return () => {
		for (const [key, descriptor] of saved) {
			if (descriptor) Object.defineProperty(proto, key, descriptor);
			else Reflect.deleteProperty(proto, key);
		}
	};
}

/** The host's app list, which the fake backend does not carry by default. */
const BACKEND = {
	appState: {
		getApps: async () => [
			[{ id: SAMPLE_APPS.invoiceAi }, { name: "Invoice AI" }],
			[{ id: SAMPLE_APPS.warehouseScanner }, { name: "Warehouse Scanner" }],
		],
	},
} as unknown as MountDevicesOptions["backend"];

const ON_EDGE: DevicesRoute = {
	screen: "device",
	deviceId: SAMPLE_IDS.edge,
	tab: "overview",
};

async function mountRail(
	options: MountDevicesOptions = {},
	route: DevicesRoute = ON_EDGE,
	scope: DevicesScope = ACCOUNT,
) {
	const { navigations, navigate } = recorder();
	const mounted = await mountDevices(
		<DeviceRail
			scope={scope}
			route={route}
			mode="docked"
			onNavigate={navigate}
		/>,
		{ backend: BACKEND, ...options },
	);
	return { mounted, navigations };
}

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
	window.localStorage.clear();
});
afterAll(dom.restore);

describe("rail model", () => {
	test("each device lands once, in the first group that fits", () => {
		const groups = groupRailDevices(SAMPLE);
		expect(groups.attention.map(idOf)).toEqual([
			"warehouse-pi",
			"studio-mac-mini",
			"edge-berlin-01",
			"cold-storage-nas",
		]);
		expect(groups.setup).toEqual([]);
		expect(groups.offline).toEqual([]);
		expect(groups.shared.map(idOf)).toEqual(["lab-gpu-02"]);
		expect(groups.unknown).toEqual([]);
		expect(groups.revoked.map(idOf)).toEqual(["old-kiosk", "partner-edge"]);
		const total = Object.values(groups).reduce((sum, rows) => {
			return sum + rows.length;
		}, 0);
		expect(total).toBe(SAMPLE.length);
	});

	test("chips and the annunciator filter narrow the list", () => {
		expect(matching("mine")).toHaveLength(5);
		expect(matching("shared")).toEqual(["lab-gpu-02", "partner-edge"]);
		expect(matching("locked")).toEqual(["cold-storage-nas", "lab-gpu-02"]);
		expect(matching("all", "offline")).toEqual(["warehouse-pi"]);
		expect(matching("all", "shared")).toEqual(["lab-gpu-02", "partner-edge"]);
		expect(matching("mine", "shared")).toEqual([]);
		expect(matching("mine", "critical")).toHaveLength(1);
	});

	test("groups cap at 8 rows, Healthy collapses above 12, Revoked starts closed", () => {
		const many = Array.from({ length: 30 }, (_, index) => {
			return device(`healthy-${String(index).padStart(2, "0")}`);
		});
		const groups = groupRailDevices([...many, ...warned(70, 2), ...SAMPLE]);
		const layout = { open: {}, caps: {}, packages: PACKAGES };
		const items = railItems(groups, layout);
		expect(headsOf(items).map(groupOfHead)).toEqual([
			"attention",
			"setup",
			"offline",
			"shared",
			"healthy",
			"revoked",
		]);
		expect(headOf(items, "healthy")).toMatchObject({ open: false });
		expect(headOf(items, "revoked")).toMatchObject({ open: false });
		expect(headOf(items, "setup")).toMatchObject({ count: 2 });
		expect(countOf(items, "device")).toBe(9);
		expect(moreOf(items)[0]).toEqual({
			kind: "more",
			group: "attention",
			hidden: 50,
		});
		const expanded = railItems(groups, {
			...layout,
			caps: { attention: 58 },
			currentId: "healthy-03",
		});
		expect(moreOf(expanded).map(hiddenOf)).toContain(16);
		expect(headOf(expanded, "healthy")).toMatchObject({ open: true });
	});

	test("an empty group keeps its head and says why; Status unknown hides", () => {
		const items = railItems(groupRailDevices([device("solo")]), {
			open: {},
			caps: {},
			packages: [],
		});
		expect(noteGroups(items)).toEqual([
			"attention",
			"setup",
			"offline",
			"shared",
			"revoked",
		]);
		expect(headOf(items, "unknown")).toBeUndefined();
	});
});

describe("device rail", () => {
	test("docked: head, home row, groups in order, notes for empty groups, foot", async () => {
		const { node } = view();
		const { container } = await dom.render(node);
		const aside = container.querySelector("aside") as HTMLElement;
		expect(aside.getAttribute("aria-label")).toBe("Devices");
		expect(aside.className).toContain("w-70");
		expect(container.innerHTML).not.toContain("fixed");
		expect(groupIds(container)).toEqual([
			"attention",
			"setup",
			"offline",
			"shared",
			"healthy",
			"revoked",
		]);
		expect(rowNames(container)).toEqual([
			"warehouse-pi",
			"studio-mac-mini",
			"edge-berlin-01",
			"cold-storage-nas",
			"factory-line-3",
			"test-vm",
			"lab-gpu-02",
		]);
		expect(textOf(container, "[data-rail-meta]")).toBe(
			"7 devices · 2 locked · 2 setup packages",
		);
		expect(textOf(container, "[data-rail-note=offline]")).toBe(
			"warehouse-pi is offline too. It's listed under Needs attention because of open items.",
		);
		expect(textOf(container, "[data-rail-note=healthy]")).toBe(
			"No device of yours is free of attention items right now.",
		);
		const home = container.querySelector("[data-rail-home]") as HTMLElement;
		expect(home.textContent).toContain("Fleet overview");
		expect(home.textContent).toContain("15");
		expect(home.getAttribute("aria-current")).toBe("page");
		expect(byRole("button", "Unlock several…")).toBeTruthy();
		expect(
			byRole("button", "Lock all").getAttribute("aria-disabled"),
		).toBeNull();
	});

	test("a row opens the device through the router; the home row opens the fleet", async () => {
		const { node, navigations } = view({
			route: { screen: "device", deviceId: "edge-berlin-01", tab: "overview" },
		});
		const { container } = await dom.render(node);
		const current = container.querySelector("[aria-current=page]");
		expect(current?.textContent).toContain("edge-berlin-01");
		await click(byText("warehouse-pi").closest("a") as HTMLElement);
		await click(container.querySelector("[data-rail-home]") as HTMLElement);
		expect(navigations.map(routeOf)).toEqual([
			{ screen: "device", deviceId: "warehouse-pi", tab: "overview" },
			{ screen: "fleet", view: "devices" },
		]);
	});

	test("the filter matches name, ID, service and app; chips narrow; packages hide while narrowed", async () => {
		const { node } = view();
		const { container } = await dom.render(node);
		const filter = byRole("textbox", "Filter devices");
		await typeInto(filter, "invoice");
		expect(rowNames(container)).toEqual(["edge-berlin-01"]);
		await typeInto(filter, "warehouse");
		expect(rowNames(container)).toEqual(["warehouse-pi"]);
		await typeInto(filter, "");
		await click(byRole("button", "Locked"));
		expect(rowNames(container)).toEqual(["cold-storage-nas", "lab-gpu-02"]);
		expect(byRole("button", "Locked").getAttribute("aria-pressed")).toBe(
			"true",
		);
		await click(byRole("button", "Shared"));
		expect(rowNames(container)).toEqual(["lab-gpu-02"]);
		await click(byRole("button", "All"));
		expect(rowNames(container)).toContain("factory-line-3");
	});

	test("an annunciator filter shows as a clearable chip and clears through the route", async () => {
		const { node, navigations } = view({
			route: { screen: "fleet", view: "devices", filter: "critical" },
		});
		const { container } = await dom.render(node);
		expect(rowNames(container)).toEqual(["warehouse-pi"]);
		await click(byRole("button", /Clear filter Critical/));
		expect(navigations).toEqual([
			{
				route: { screen: "fleet", view: "devices", filter: undefined },
				replace: true,
			},
		]);
	});

	test("groups collapse, remember the choice and reveal more rows in steps", async () => {
		const { node } = view({ devices: warned(20, 2), packages: [] });
		const { container } = await dom.render(node);
		expect(rowNames(container)).toHaveLength(8);
		await clickByText("Show 12 more");
		expect(rowNames(container)).toHaveLength(20);
		const head = container.querySelector(
			"[data-rail-group=attention]",
		) as HTMLElement;
		await click(head);
		expect(head.getAttribute("aria-expanded")).toBe("false");
		expect(rowNames(container)).toHaveLength(0);
		expect(
			window.localStorage.getItem("flow-like.devices.rail-groups"),
		).toContain('"attention":false');
	});

	test("older hub: not-owned rows are labelled 'Shared or cloud approvals'", async () => {
		const { node } = view({
			devices: [device("mine"), device("theirs", { relationship: "unknown" })],
			packages: [],
		});
		const { container } = await dom.render(node);
		expect(byRole("button", "Shared or cloud approvals")).toBeTruthy();
		expect(
			container.querySelector("[data-rail-group=shared]")?.textContent,
		).toContain("Shared or cloud approvals");
	});

	test("foot actions call out; Lock all is disabled while nothing is unlocked", async () => {
		const calls: string[] = [];
		const onUnlockSeveral = () => {
			calls.push("several");
		};
		const onLockAll = () => {
			calls.push("lock-all");
		};
		const { node } = view({ unlockedCount: 0, onUnlockSeveral, onLockAll });
		await dom.render(node);
		await click(byRole("button", "Unlock several…"));
		const lockAll = byRole("button", "Lock all");
		expect(lockAll.getAttribute("aria-disabled")).toBe("true");
		await click(lockAll);
		expect(calls).toEqual(["several"]);
	});

	test("a list that never loaded is not shown as empty", async () => {
		const { node } = view({
			devices: [],
			packages: [],
			unavailable: <p data-probe="">Reading the device list…</p>,
		});
		const { container } = await dom.render(node);
		expect(container.querySelector("[data-probe]")).not.toBeNull();
		expect(groupIds(container)).toEqual([]);
	});

	test("app scope: the home row is the app's page", async () => {
		const { node, navigations } = view({
			scope: { kind: "app", appId: "app_invoice_ai" },
			route: { screen: "app-devices", by: "device" },
		});
		const { container } = await dom.render(node);
		const home = container.querySelector("[data-rail-home]") as HTMLElement;
		expect(home.textContent).toContain("This app on devices");
		expect(home.getAttribute("href")).toContain("/library/config/devices");
		await click(home);
		expect(navigations[0]?.route).toEqual({
			screen: "app-devices",
			by: "device",
		});
	});

	test("overlay: a left sheet with a close button; choosing a device closes it", async () => {
		const changes: boolean[] = [];
		const onOpenChange = (open: boolean) => {
			changes.push(open);
		};
		const { node, navigations } = view({
			mode: "overlay",
			open: true,
			onOpenChange,
		});
		await dom.render(node);
		await settle();
		const sheet = byRole("dialog", "Devices");
		expect(queryByRole("textbox", "Filter devices", sheet)).not.toBeNull();
		await click(byText("studio-mac-mini").closest("a") as HTMLElement);
		expect(navigations[0]?.route).toMatchObject({
			screen: "device",
			deviceId: "studio-mac-mini",
		});
		await click(byRole("button", "Hide devices"));
		expect(changes).toEqual([false, false]);
	});

	test("overlay closed: nothing is mounted", async () => {
		const { node } = view({ mode: "overlay", open: false });
		await dom.render(node);
		expect(queryByRole("dialog")).toBeNull();
		expect(allByRole("link")).toHaveLength(0);
	});
});

const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\b[GD]\d{1,2}\b/;

describe("device rail over the workspace", () => {
	test("groups the fleet by what needs you, with each row's reason, state and agent", async () => {
		const { mounted } = await mountRail();
		const { container } = mounted;
		expect(groupIds(container)).toEqual([
			"attention",
			"setup",
			"offline",
			"shared",
			"healthy",
			"revoked",
		]);
		expect(rowNames(container)).toEqual([
			"warehouse-pi",
			"studio-mac-mini",
			"edge-berlin-01",
			"cold-storage-nas",
			"factory-line-3",
			"test-vm",
			"lab-gpu-02",
		]);
		const rows = Array.from(container.querySelectorAll("[data-rail-row]"));
		const texts = rows.map(contentOf);
		expect(texts[0]).toContain("Crashing · offline · 0.9.2");
		expect(texts[0]).toContain("2 critical");
		expect(texts[2]).toContain("Certificate expiring · online · 0.9.4");
		const current = rows[2] as HTMLElement;
		expect(current.getAttribute("aria-current")).toBe("page");
		expect(texts[3]).toContain("Never checked in · no check-in");
		expect(texts[4]).toContain("package");
		expect(textOf(container, "[data-rail-meta]")).toBe(
			"7 devices · 3 locked · 2 setup packages",
		);
		expect(textOf(container, "[data-rail-home]")).toContain("15");
		expect(textOf(container, "[data-rail-group=revoked]")).toContain("2");
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
		expect(container.innerHTML).not.toContain("fixed");
	});

	test("rows and setup packages open their page", async () => {
		const { mounted, navigations } = await mountRail();
		await click(byText("warehouse-pi").closest("a") as HTMLElement);
		await click(byText("factory-line-3").closest("a") as HTMLElement);
		await mounted.settle();
		expect(navigations[0]?.route).toEqual({
			screen: "device",
			deviceId: SAMPLE_IDS.warehouse,
			tab: "overview",
		});
		expect(navigations[1]?.route).toMatchObject({ screen: "setup" });
		expect(
			(navigations[1]?.route as { enrollmentId?: string }).enrollmentId,
		).toBeTruthy();
	});

	test("the filter finds a device by its service and by its app's name", async () => {
		const { mounted } = await mountRail();
		const filter = byRole("textbox", "Filter devices");
		await typeInto(filter, "scanner");
		expect(rowNames(mounted.container)).toEqual(["warehouse-pi"]);
		await typeInto(filter, "Invoice");
		expect(rowNames(mounted.container)).toEqual(["edge-berlin-01"]);
		await typeInto(filter, SAMPLE_IDS.lab.slice(0, 8));
		expect(rowNames(mounted.container)).toEqual(["lab-gpu-02"]);
	});

	test("Lock all closes every key session; Unlock several… asks the unlock sheet", async () => {
		const { mounted } = await mountRail();
		const { keys } = mounted.fake.workspace;
		await click(byRole("button", "Unlock several…"));
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock_several");
		await click(byRole("button", "Lock all"));
		await mounted.settle();
		expect(keys.list().map(stateOf)).not.toContain("unlocked");
		expect(byRole("button", "Lock all").getAttribute("aria-disabled")).toBe(
			"true",
		);
		expect(
			mounted.container.querySelector("[data-rail-meta]")?.textContent,
		).toContain("6 locked");
	});

	test("app scope: the home row is the app's page and counts the app's items", async () => {
		const { mounted } = await mountRail(
			{},
			{ screen: "app-devices", by: "device" },
			{ kind: "app", appId: SAMPLE_APPS.fieldNotes },
		);
		const home = mounted.container.querySelector("[data-rail-home]");
		expect(home?.textContent).toContain("This app on devices");
		expect(home?.getAttribute("aria-current")).toBe("page");
		expect(home?.textContent).not.toContain("15");
	});

	test("older hub: rows that aren't yours read 'Shared or cloud approvals'", async () => {
		const { mounted } = await mountRail({ hubVersion: "old" });
		const { container } = mounted;
		expect(byRole("button", "Shared or cloud approvals")).toBeTruthy();
		expect(
			container.querySelector("[data-rail-group=shared]")?.textContent,
		).toContain("Shared or cloud approvals");
		expect(rowNames(container)).toContain("lab-gpu-02");
		expect(rowNames(container)).toContain("warehouse-pi");
		expect(container.querySelector("[data-kind=error]")).toBeNull();
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("a list that hasn't arrived reads as loading, never as an empty fleet", async () => {
		const fake = await createFakeWorkspace(undefined, {});
		const release = fake.api.hold({ method: "GET", path: "devices" });
		const { mounted } = await mountRail({ fake });
		expect(byRole("status").textContent).toContain("Reading the device list…");
		expect(groupIds(mounted.container)).toEqual([]);
		release();
		await mounted.settle();
		expect(rowNames(mounted.container)).toContain("edge-berlin-01");
	});

	test("a list that can't be read says why instead of showing no devices", async () => {
		const fake = await createFakeWorkspace(undefined, {});
		fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({
				status: 403,
				code: "FORBIDDEN",
				message: "refused",
			}),
		);
		const { mounted } = await mountRail({ fake });
		await mounted.settle();
		const state = mounted.container.querySelector("[data-kind=error]");
		expect(state?.textContent).toContain("Couldn't read the device list");
		expect(groupIds(mounted.container)).toEqual([]);
		expect(mounted.container.textContent).not.toMatch(MACHINE_WORDS);
	});
});

describe("device rail at 200 devices", () => {
	let restore = () => {};
	beforeAll(() => {
		restore = fakeViewport(600, 280);
	});
	afterAll(() => {
		restore();
	});

	test("is windowed: only the rows in view are in the document", async () => {
		const fleet = warned(200, 3, "edge");
		const { node } = view({ devices: fleet, packages: [] });
		const { container } = await dom.render(node);
		await settle();
		const body = container.querySelector("[data-rail-body]") as HTMLElement;
		expect(body.hasAttribute("data-windowed")).toBe(true);
		expect(rowNames(container).length).toBeGreaterThan(0);
		expect(rowNames(container).length).toBeLessThanOrEqual(8);
		await clickByText("Show 50 more");
		await settle();
		const rendered = rowNames(container).length;
		expect(rendered).toBeGreaterThan(8);
		expect(rendered).toBeLessThan(40);
		expect(rowNames(container)).not.toContain("edge-057");
		body.scrollTop = 2200;
		await fire(body, new window.Event("scroll") as unknown as Event);
		await settle();
		expect(rowNames(container)).toContain("edge-057");
		expect(rowNames(container)).not.toContain("edge-000");
		expect(byText("Show 50 more")).toBeTruthy();
		expect(container.querySelector("[data-rail-meta]")?.textContent).toContain(
			"200 devices",
		);
	});

	test("the generated 200-device fleet renders windowed through the workspace", async () => {
		const { mounted } = await mountRail({
			seed: generateFleet(200).input,
			unlock: "none",
		});
		const { container } = mounted;
		const body = container.querySelector("[data-rail-body]") as HTMLElement;
		expect(body.hasAttribute("data-windowed")).toBe(true);
		expect(container.querySelector("[data-rail-meta]")?.textContent).toContain(
			"200 devices",
		);
		const rendered = rowNames(container).length;
		expect(rendered).toBeGreaterThan(0);
		expect(rendered).toBeLessThan(40);
		expect(
			container.querySelector("[data-rail-group=attention]")?.textContent,
		).toMatch(/\d+/);
	});
});
