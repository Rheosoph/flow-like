import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_PEOPLE,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { ManagementPolicy } from "../../../../lib/device-management/types";
import {
	byRole,
	click,
	clickByText,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { MACHINE_WORDS, mountAccess } = await import(
	"../access/access-test-kit"
);
const { DeviceAccessTab } = await import("./access-tab");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
	globalThis.localStorage?.clear();
});
afterAll(dom.restore);

const { jonas: JONAS } = SAMPLE_PEOPLE;

function Routed({ deviceId }: Readonly<{ deviceId: string }>) {
	const { route, scope } = useDevicesRoute();
	return <DeviceAccessTab route={route} scope={scope} deviceId={deviceId} />;
}

const mount = (
	deviceId: string,
	options: Parameters<typeof mountAccess>[1] = {},
) =>
	mountAccess(<Routed deviceId={deviceId} />, {
		search: `device=${deviceId}&tab=access`,
		...options,
	});

const policyOf = (fake: FakeWorkspace, deviceId: string) =>
	fake.hub.policies.get(deviceId)?.policy as ManagementPolicy;

describe("Device › Access (owner)", () => {
	test("the rules, the people and the actions of this device", async () => {
		const { container } = await mount(SAMPLE_IDS.edge);
		const rules = container.querySelector(
			"#device-access-rules",
		) as HTMLElement;
		expect(byRole("heading", "Access rules v5", rules)).toBeTruthy();
		expect(rules.textContent).toContain("Saved v5");
		expect(rules.textContent).toContain("device has v5");
		expect(rules.textContent).toContain("Active on device");
		expect(rules.textContent).toContain("2 of 24");
		expect(rules.textContent).toContain(
			"Shared access ends and retained history pauses for everyone, you included.",
		);
		expect(rules.querySelector("[data-stamp]")).toBeTruthy();
		const people = container.querySelector(
			"#device-access-people",
		) as HTMLElement;
		expect(people.querySelectorAll("[data-grant]")).toHaveLength(2);
		expect(people.querySelector("[data-stamp]")?.textContent).toContain(
			"access rules v5",
		);
		expect(
			byRole("link", "People on all your devices", people).getAttribute("href"),
		).toBe("/settings/devices?view=access&tab=people");
		expect(container.querySelectorAll("[data-dv-primary]").length).toBe(0);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("Add people opens in place with this device chosen", async () => {
		const { container } = await mount(SAMPLE_IDS.edge);
		await click(
			byRole(
				"button",
				"Add people…",
				container.querySelector("#device-access-rules") as HTMLElement,
			),
		);
		const sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 1 of 5 · Devices");
		expect(
			sheet
				.querySelector(
					`[data-device-choice="${SAMPLE_IDS.edge}"] [role=checkbox]`,
				)
				?.getAttribute("aria-checked"),
		).toBe("true");
		expect(
			byRole("button", "Continue", sheet).getAttribute("aria-disabled"),
		).toBeNull();
	});

	test("change permissions: the review lists what is removed, and an unchanged draft can't be saved", async () => {
		const mounted = await mount(SAMPLE_IDS.edge);
		const { container, fake } = mounted;
		const people = container.querySelector(
			"#device-access-people",
		) as HTMLElement;
		await click(byRole("button", "More for Jonas Weber", people));
		await clickByText("Change permissions…", inPortal("menu"));
		let sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Change Jonas Weber's permissions");
		expect(sheet.textContent).toContain("Step 1 of 3 · Permissions");
		expect(
			byRole("button", "Continue", sheet).getAttribute("aria-disabled"),
		).toBe("true");
		expect(sheet.textContent).toContain(
			"Change a permission, the scope or the end date first.",
		);
		await click(
			sheet.querySelector(
				"[data-permission=logs] [role=checkbox]",
			) as HTMLElement,
		);
		await click(byRole("button", "Continue", inPortal("dialog")));

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 2 of 3 · Review");
		expect(
			sheet
				.querySelector("[data-change-kind]")
				?.getAttribute("data-change-kind"),
		).toBe("changed");
		const removed = sheet.querySelector("[data-k=removed]") as HTMLElement;
		expect(removed.textContent).toContain("Read logs");
		expect(sheet.querySelector("[data-k=added]")).toBeNull();
		expect(sheet.textContent).toContain("Change them back the same way.");
		await click(byRole("button", "Save access rules", sheet));
		await mounted.settle();
		const jonas = policyOf(fake, SAMPLE_IDS.edge).grants.find(
			(grant) => grant.user_id === JONAS,
		);
		expect(jonas?.capabilities).toEqual([
			"status",
			"deploy",
			"start",
			"stop",
			"restart",
			"scale",
		]);
		// The end stays where it was unless another one is chosen.
		expect(jonas?.expires_at).toBe(1_790_820_000);
	});

	test("locked: the hub's facts stay, the people need the keys, and gated controls send nothing", async () => {
		const { container, fake } = await mount(SAMPLE_IDS.edge, {
			unlock: "none",
		});
		const rules = container.querySelector(
			"#device-access-rules",
		) as HTMLElement;
		expect(rules.textContent).toContain("Saved v5");
		expect(rules.textContent).toContain("Shows once unlocked");
		const people = container.querySelector(
			"#device-access-people",
		) as HTMLElement;
		expect(people.querySelector("[data-gate=locked]")).toBeTruthy();
		expect(people.querySelector("[data-grant]")).toBeNull();
		expect(people.textContent).not.toContain("Nobody else has access");
		const renew = byRole("button", "Renew access rules…", rules);
		expect(renew.getAttribute("aria-disabled")).toBe("true");
		const writes = fake.api.writes().length;
		await click(renew);
		expect(fake.api.writes()).toHaveLength(writes);
		expect(queryByRole("alertdialog")).toBeNull();
	});

	test("a device nobody else can reach says so instead of showing an empty table", async () => {
		const { container } = await mount(SAMPLE_IDS.warehouse);
		expect(container.querySelector("[data-rules=none]")?.textContent).toContain(
			"Not shared. Only you can reach this device.",
		);
		expect(container.textContent).toContain("Nobody else has access");
		expect(queryByRole("button", "Renew access rules…")).toBeNull();
	});
});

describe("Device › Access (recipient and cloud-only)", () => {
	test("your access: owner, scope, permissions, end and what you can do about it", async () => {
		const { container } = await mount(SAMPLE_IDS.lab);
		const block = container.querySelector("#device-your-access") as HTMLElement;
		expect(block.textContent).toContain("Mira Novak");
		expect(block.querySelector("[data-permissions]")?.textContent).toBe(
			"Custom · 6 permissions",
		);
		expect(block.querySelector("[data-scope=app]")?.textContent).toBe(
			"App Invoice AI",
		);
		expect(block.textContent).toContain("Active");
		expect(byRole("button", "Ask to renew", block)).toBeTruthy();
		expect(byRole("button", "Remove from this computer…", block)).toBeTruthy();
		expect(
			byRole("link", "Everything shared with you", block).getAttribute("href"),
		).toBe("/settings/devices?view=access&tab=shared");
		expect(container.querySelector("#device-access-rules")).toBeNull();
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
	});

	test("an older hub: the interim says what shows after unlocking, without an error", async () => {
		const { container, fake } = await mount(SAMPLE_IDS.lab, {
			hubVersion: "old",
		});
		const block = container.querySelector("#device-your-access") as HTMLElement;
		expect(block.querySelector("[data-own-access=unknown]")?.textContent).toBe(
			"Shows once unlocked",
		);
		expect(byRole("button", "Unlock…", block)).toBeTruthy();
		expect(container.querySelector("[role=alert]")).toBeNull();
		expect(
			fake.api.sent("GET", /management\/my-access/).length,
		).toBeLessThanOrEqual(1);
	});

	test("a revoked device and a device where I only approved cloud access show the cloud approvals alone", async () => {
		const revoked = await mount(SAMPLE_IDS.oldKiosk);
		expect(revoked.container.querySelector("#device-access-rules")).toBeNull();
		expect(revoked.container.querySelector("#device-your-access")).toBeNull();
		expect(revoked.container.textContent).not.toContain("Add people…");
		await cleanupDevices();
		const partner = await mount(SAMPLE_IDS.partner);
		expect(partner.container.querySelector("#device-access-rules")).toBeNull();
		expect(partner.container.querySelector("#device-your-access")).toBeNull();
	});
});
