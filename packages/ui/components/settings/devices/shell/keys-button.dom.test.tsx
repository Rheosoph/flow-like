import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import {
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
const { KeysButton, KeysButtonView } = await import("./keys-button");
const { useOverlayStore } = await import("../workspace");

async function mountButton(options: MountDevicesOptions = {}) {
	const navigations: DevicesRoute[] = [];
	const mounted = await mountDevices(
		<KeysButton
			scope={{ kind: "account" }}
			onNavigate={(route) => {
				navigations.push(route);
			}}
		/>,
		options,
	);
	return { mounted, navigations };
}

const sessionIds = () =>
	Array.from(
		byRole("dialog", "Key sessions").querySelectorAll("[data-key-session]"),
		(el) => el.getAttribute("data-key-session"),
	);

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

describe("keys button", () => {
	test("says how many devices are unlocked", async () => {
		const { container } = await dom.render(<KeysButtonView unlocked={3} />);
		const button = byRole("button", "Key sessions: 3 unlocked");
		expect(button.textContent).toContain("3 unlocked");
		expect(button.getAttribute("aria-haspopup")).toBe("dialog");
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});

	test("says Locked when no keys are open", async () => {
		await dom.render(<KeysButtonView unlocked={0} />);
		const button = byRole("button", "Key sessions: all locked");
		expect(button.textContent).toContain("Locked");
	});

	test("opens the key sessions popover", async () => {
		const changes: boolean[] = [];
		const { rerender } = await dom.render(
			<KeysButtonView
				unlocked={1}
				open={false}
				onOpenChange={(open) => changes.push(open)}
			>
				<p>sessions</p>
			</KeysButtonView>,
		);
		expect(queryByRole("dialog")).toBeNull();
		await click(byRole("button", /Key sessions/));
		expect(changes).toEqual([true]);
		await rerender(
			<KeysButtonView unlocked={1} open onOpenChange={() => {}}>
				<p>sessions</p>
			</KeysButtonView>,
		);
		await settle();
		expect(byRole("dialog", "Key sessions").textContent).toBe("sessions");
	});
});

describe("keys button over the workspace", () => {
	test("counts the open key sessions and lists every device with keys here, open ones first", async () => {
		const { mounted } = await mountButton();
		await click(byRole("button", "Key sessions: 3 unlocked"));
		await mounted.settle();
		const popover = byRole("dialog", "Key sessions");
		const ids = sessionIds();
		expect(ids.slice(0, 3).sort()).toEqual(
			[SAMPLE_IDS.edge, SAMPLE_IDS.studio, SAMPLE_IDS.warehouse].sort(),
		);
		expect(ids).toContain(SAMPLE_IDS.lab);
		expect(popover.textContent).toContain("edge-berlin-01");
		expect(popover.textContent).toContain("Live · relayed");
		expect(popover.textContent).toContain("Locked");
		expect(popover.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});

	test("Lock closes one device's keys; Lock all closes the rest", async () => {
		const { mounted } = await mountButton();
		const { keys } = mounted.fake.workspace;
		await click(byRole("button", /^Key sessions/));
		await mounted.settle();
		await click(byRole("button", "Lock edge-berlin-01"));
		await mounted.settle();
		expect(keys.snapshot(SAMPLE_IDS.edge).state).toBe("locked");
		expect(queryByRole("dialog")).toBeNull();
		await click(byRole("button", "Key sessions: 2 unlocked"));
		await mounted.settle();
		await click(byRole("button", "Lock all"));
		await mounted.settle();
		expect(byRole("button", "Key sessions: all locked")).toBeTruthy();
		expect(keys.list().some((row) => row.state === "unlocked")).toBe(false);
	});

	test("Unlock… and Unlock several… ask the unlock sheet; Keys & recovery is a place", async () => {
		const { mounted, navigations } = await mountButton();
		const button = byRole("button", /^Key sessions/);
		await click(button);
		await mounted.settle();
		await click(byRole("button", "Unlock lab-gpu-02…"));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
		});
		expect(queryByRole("dialog")).toBeNull();

		await click(button);
		await mounted.settle();
		await click(byRole("button", "Unlock several…"));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock_several");

		await click(button);
		await mounted.settle();
		await click(byRole("button", "Keys & recovery"));
		expect(navigations).toEqual([{ screen: "keys" }]);
	});

	test("nothing unlocked: the button says Locked and Lock all is disabled", async () => {
		const { mounted } = await mountButton({ unlock: "none" });
		await click(byRole("button", "Key sessions: all locked"));
		await mounted.settle();
		expect(byRole("button", "Lock all").getAttribute("aria-disabled")).toBe(
			"true",
		);
		expect(sessionIds().length).toBeGreaterThan(0);
	});

	test("older hub: key sessions are this computer's and read the same", async () => {
		const { mounted } = await mountButton({ hubVersion: "old" });
		await click(byRole("button", "Key sessions: 3 unlocked"));
		await mounted.settle();
		expect(sessionIds().length).toBeGreaterThan(3);
	});
});
