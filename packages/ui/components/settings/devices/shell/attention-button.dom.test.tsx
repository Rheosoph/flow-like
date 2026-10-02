import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
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

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { AttentionButton, AttentionButtonView, CountPill } = await import(
	"./attention-button"
);
const { useOverlayStore } = await import("../workspace");

const ACCOUNT: DevicesScope = { kind: "account" };
const ON_DEVICE: DevicesRoute = {
	screen: "device",
	deviceId: SAMPLE_IDS.edge,
	tab: "overview",
};

/** The host's app list, which the fake backend does not carry by default. */
const BACKEND = {
	appState: {
		getApps: async () => [
			[{ id: SAMPLE_APPS.invoiceAi }, { name: "Invoice AI" }],
			[{ id: SAMPLE_APPS.warehouseScanner }, { name: "Warehouse Scanner" }],
		],
	},
} as unknown as MountDevicesOptions["backend"];

interface Nav {
	route: DevicesRoute;
	replace: boolean;
}

async function mountButton(
	scope: DevicesScope,
	route: DevicesRoute,
	options: MountDevicesOptions = {},
) {
	const navigations: Nav[] = [];
	const mounted = await mountDevices(
		<AttentionButton
			scope={scope}
			route={route}
			appName="Field Notes"
			onNavigate={(target, how) => {
				navigations.push({ route: target, replace: how?.replace ?? false });
			}}
		/>,
		{ backend: BACKEND, ...options },
	);
	return { mounted, navigations };
}

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

describe("attention button", () => {
	test("splits the count: critical apart, square-cornered, with its glyph", async () => {
		const { container } = await dom.render(
			<AttentionButtonView critical={2} total={15}>
				<p>popover body</p>
			</AttentionButtonView>,
		);
		const button = byRole("button", "Attention: 2 critical, 15 in total");
		const critical = container.querySelector("[data-count=critical]");
		const total = container.querySelector("[data-count=total]");
		expect(critical?.textContent).toBe("2");
		expect(critical?.className).toContain("rounded-sm");
		expect(critical?.querySelector("svg")).not.toBeNull();
		expect(total?.textContent).toBe("15");
		expect(total?.className).toContain("rounded-full");
		expect(button.getAttribute("aria-haspopup")).toBe("dialog");
		expect(button.querySelector("[data-dv-primary]")).toBeNull();
		expect(button.hasAttribute("data-dv-primary")).toBe(false);
	});

	test("no critical items: only the total shows", async () => {
		const { container } = await dom.render(
			<AttentionButtonView critical={0} total={3} />,
		);
		expect(container.querySelector("[data-count=critical]")).toBeNull();
		expect(byRole("button", "Attention: 0 critical, 3 in total")).toBeTruthy();
	});

	test("the label hides in phone chrome; the counts stay", async () => {
		const { container } = await dom.render(
			<AttentionButtonView critical={1} total={4} />,
		);
		const label = Array.from(container.querySelectorAll("span")).find(
			(el) => el.textContent === "Attention",
		);
		expect(label?.className).toContain("@max-[720px]/devices:hidden");
		expect(
			container.querySelector("[data-count=total]")?.className,
		).not.toContain("hidden");
	});

	test("opens its popover with the list inside", async () => {
		const changes: boolean[] = [];
		const { rerender } = await dom.render(
			<AttentionButtonView
				critical={1}
				total={2}
				open={false}
				onOpenChange={(open) => changes.push(open)}
			>
				<p>popover body</p>
			</AttentionButtonView>,
		);
		expect(queryByRole("dialog")).toBeNull();
		await click(byRole("button", /Attention/));
		expect(changes).toEqual([true]);
		await rerender(
			<AttentionButtonView critical={1} total={2} open onOpenChange={() => {}}>
				<p>popover body</p>
			</AttentionButtonView>,
		);
		await settle();
		expect(byRole("dialog", "Needs you").textContent).toBe("popover body");
	});

	test("on the fleet overview it jumps to Needs you instead", async () => {
		let jumps = 0;
		await dom.render(
			<AttentionButtonView
				critical={0}
				total={7}
				onDirect={() => {
					jumps += 1;
				}}
			>
				<p>popover body</p>
			</AttentionButtonView>,
		);
		const button = byRole("button", /Attention/);
		expect(button.getAttribute("aria-haspopup")).toBeNull();
		await click(button);
		expect(jumps).toBe(1);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("app scope names the app", async () => {
		await dom.render(
			<AttentionButtonView critical={1} total={2} appName="Invoice AI" />,
		);
		expect(
			byRole("button", "Attention in Invoice AI: 1 critical, 2 in total"),
		).toBeTruthy();
	});

	test("the count pill is decoration: the button's name carries the numbers", async () => {
		const { container } = await dom.render(<CountPill count={4} />);
		expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe(
			"true",
		);
	});
});

const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\b[GD]\d{1,2}\b/;

describe("attention button over the workspace", () => {
	test("counts the fleet's open items and lists the five most severe", async () => {
		const { mounted, navigations } = await mountButton(ACCOUNT, ON_DEVICE);
		await click(byRole("button", "Attention: 2 critical, 15 in total"));
		await mounted.settle();
		const writes = mounted.fake.api.writes().length;
		const popover = byRole("dialog", "Needs you");
		const items = Array.from(popover.querySelectorAll("[data-attention]"));
		expect(items).toHaveLength(5);
		expect(items.map((el) => el.getAttribute("data-sev"))).toEqual([
			"critical",
			"critical",
			"warning",
			"warning",
			"warning",
		]);
		expect(byRole("heading", /Needs you/).textContent).toContain("15");
		expect(byRole("heading", /Needs you/).textContent).toContain("2 critical");
		expect(popover.textContent).toContain("warehouse-pi");
		expect(popover.textContent).not.toMatch(MACHINE_WORDS);
		expect(popover.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
		await click(byRole("button", "Show all 15 on Fleet overview"));
		await mounted.settle();
		expect(navigations).toEqual([
			{
				route: { screen: "fleet", view: "devices", focus: "attention" },
				replace: false,
			},
		]);
		expect(queryByRole("dialog")).toBeNull();
		expect(mounted.fake.api.writes()).toHaveLength(writes);
	});

	test("an item's action runs from the popover: a fix opens its overlay, a place is navigated to", async () => {
		const { mounted, navigations } = await mountButton(ACCOUNT, ON_DEVICE);
		await click(byRole("button", /^Attention/));
		await mounted.settle();
		await click(allByRole("button", "Diagnose")[0]);
		await mounted.settle();
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "diagnose",
			deviceId: SAMPLE_IDS.warehouse,
		});
		expect(queryByRole("dialog")).toBeNull();
		expect(navigations).toEqual([]);

		await click(byRole("button", /^Attention/));
		await mounted.settle();
		await click(byRole("button", "Review change"));
		await mounted.settle();
		expect(navigations).toHaveLength(1);
		expect(navigations[0]?.route).toMatchObject({
			screen: "service",
			deviceId: SAMPLE_IDS.studio,
		});
		expect(queryByRole("dialog")).toBeNull();
	});

	test("on the fleet overview it puts the focus on Needs you and opens nothing", async () => {
		const fleet: DevicesRoute = {
			screen: "fleet",
			view: "devices",
			filter: "critical",
		};
		const { mounted, navigations } = await mountButton(ACCOUNT, fleet);
		await click(byRole("button", /^Attention/));
		await mounted.settle();
		expect(navigations).toEqual([
			{ route: { ...fleet, focus: "attention" }, replace: true },
		]);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("app scope: counts and lists this app's items, and links to both lists", async () => {
		const scope: DevicesScope = { kind: "app", appId: SAMPLE_APPS.fieldNotes };
		const { mounted, navigations } = await mountButton(scope, {
			screen: "app-devices",
			by: "device",
		});
		const button = byRole("button", /^Attention in Field Notes: /);
		const total = Number(
			mounted.container.querySelector("[data-count=total]")?.textContent,
		);
		expect(total).toBeGreaterThan(0);
		expect(total).toBeLessThan(15);
		await click(button);
		await mounted.settle();
		expect(byRole("heading", /Needs you in Field Notes/)).toBeTruthy();
		expect(
			byRole("dialog", "Needs you").querySelectorAll("[data-attention]"),
		).toHaveLength(Math.min(total, 5));
		await click(byRole("button", `Show all ${total} for Field Notes`));
		await mounted.settle();
		await click(button);
		await mounted.settle();
		await click(byRole("button", "All devices' attention"));
		expect(navigations.map((entry) => entry.route)).toEqual([
			{ screen: "app-devices", by: "device" },
			{ screen: "fleet", view: "devices", focus: "attention" },
		]);
	});

	test("older hub: the count and the list still render from what that hub can say", async () => {
		const { mounted } = await mountButton(ACCOUNT, ON_DEVICE, {
			hubVersion: "old",
		});
		const button = byRole("button", /^Attention: \d+ critical, \d+ in total$/);
		await click(button);
		await mounted.settle();
		const popover = byRole("dialog", "Needs you");
		expect(popover.querySelectorAll("[data-attention]").length).toBeGreaterThan(
			0,
		);
		expect(popover.textContent).not.toMatch(MACHINE_WORDS);
		expect(popover.querySelector("[data-kind=error]")).toBeNull();
	});
});
