import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { readDeviceVault } = await import(
	"../../../../lib/device-management/storage"
);
const { ChangePasswordSheet } = await import("./change-password-sheet");
const { rowScope, useKeyFileLog, useKeyResults } = await import("./key-store");
const { useKeysModel } = await import("./use-keys-model");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const NEW_PASSWORD = "a brand new device password";
const VAULTS = /^devices\/controller-vaults\//;

function Harness({ deviceId = EDGE }: Readonly<{ deviceId?: string }>) {
	const read = useKeysModel();
	const files = useKeyFileLog();
	const results = useKeyResults();
	const row = read.model.rows.find((entry) => entry.deviceId === deviceId);
	return (
		<>
			{read.devices.loaded ? (
				<ChangePasswordSheet
					read={read}
					files={files}
					deviceId={deviceId}
					onClose={() => undefined}
				/>
			) : null}
			<output data-result="">{results.of(rowScope(deviceId))?.text}</output>
			<output data-category="">{row?.category}</output>
		</>
	);
}

type View = Awaited<ReturnType<typeof mountDevices>>;

const FIELDS =
	"#keys-password-current, #keys-password-new, #keys-password-repeat";

function inputs(): HTMLInputElement[] {
	return Array.from(
		byRole("dialog").querySelectorAll<HTMLInputElement>(FIELDS),
	);
}

function typed(): string[] {
	return inputs().map((input) => input.value);
}

async function fill(current: string, next: string, repeat = next) {
	const [currentField, nextField, repeatField] = inputs();
	await typeInto(currentField as HTMLInputElement, current);
	await typeInto(nextField as HTMLInputElement, next);
	await typeInto(repeatField as HTMLInputElement, repeat);
	await click(byRole("button", "Change password", byRole("dialog")));
}

async function vaultBytes(view: View): Promise<string> {
	const stored = await readDeviceVault(view.fake.scope, EDGE);
	return Array.from(stored?.controllerVault ?? []).join(",");
}

function categoryOf(view: View): string | null | undefined {
	return view.container.querySelector("[data-category]")?.textContent;
}

/** Locks the device and tries a password, as the unlock sheet would. */
async function unlocks(view: View, password: string): Promise<boolean> {
	const { keys } = view.fake.workspace;
	let opened = false;
	await act(async () => {
		keys.lock(EDGE);
		opened = await keys.unlock(EDGE, password).then(
			() => true,
			() => false,
		);
	});
	await view.settle();
	return opened;
}

describe("change device password (inherits device-password-change 1–3)", () => {
	test("a mismatched repeat clears every password field and changes nothing", async () => {
		const view = await mountDevices(<Harness />);
		const before = await vaultBytes(view);
		const writes = view.fake.api.writes().length;
		await fill(view.fake.password, NEW_PASSWORD, "something else entirely");
		await view.settle();
		expect(typed()).toEqual(["", "", ""]);
		expect(byRole("dialog").textContent).toContain(
			"The two new passwords don't match.",
		);
		expect(await vaultBytes(view)).toBe(before);
		expect(view.fake.api.writes().length).toBe(writes);
	});

	test("the change works while the device is unlocked, then offers both backups and explains old copies", async () => {
		const view = await mountDevices(<Harness />);
		expect(view.fake.workspace.keys.snapshot(EDGE).state).toBe("unlocked");
		expect(byRole("dialog").textContent).toContain(
			"is unlocked and stays unlocked",
		);
		const before = await vaultBytes(view);
		await fill(view.fake.password, NEW_PASSWORD);
		await view.settle();

		const dialog = byRole("dialog");
		expect(dialog.textContent).toContain("Password changed for edge-berlin-01");
		expect(dialog.textContent).toContain(
			"Your account backup (v3) and old backup files still open with the old password until you update them.",
		);
		expect(dialog.querySelectorAll("input").length).toBe(0);
		expect(byRole("button", "Update account backup", dialog)).toBeTruthy();
		expect(byRole("button", "Download new backup file", dialog)).toBeTruthy();
		expect(dialog.querySelectorAll("[data-dv-primary]").length).toBe(1);
		expect(await vaultBytes(view)).not.toBe(before);
		expect(view.fake.workspace.keys.snapshot(EDGE).state).toBe("unlocked");
		expect(categoryOf(view)).toBe("oldpw");
		expect(dialog.textContent).not.toContain(NEW_PASSWORD);

		await click(byRole("button", "Update account backup", dialog));
		await view.settle();
		expect(view.fake.api.sent("PUT", VAULTS).length).toBe(1);
		expect(byRole("dialog").textContent).toContain(
			"Account backup updated as version 4",
		);
		expect(categoryOf(view)).toBe("synced");
		expect(queryByRole("button", "Update account backup")).toBeNull();
		expect(JSON.stringify(view.fake.api.calls)).not.toContain(NEW_PASSWORD);

		expect(await unlocks(view, view.fake.password)).toBe(false);
		expect(await unlocks(view, NEW_PASSWORD)).toBe(true);
	});

	test("a failed save reports no success and offers no backup of keys that were not committed", async () => {
		const view = await mountDevices(<Harness />);
		const before = await vaultBytes(view);
		view.fake.browser.failNextCommit();
		await fill(view.fake.password, NEW_PASSWORD);
		await view.settle();
		const dialog = byRole("dialog");
		expect(dialog.textContent).not.toContain("Password changed");
		expect(dialog.textContent).toContain(
			"This computer's storage refused the change. Nothing was saved.",
		);
		expect(queryByRole("button", "Download new backup file")).toBeNull();
		expect(queryByRole("button", "Update account backup")).toBeNull();
		expect(typed()).toEqual(["", "", ""]);
		expect(await vaultBytes(view)).toBe(before);
		expect(view.container.querySelector("[data-result]")?.textContent).toBe("");
	});

	test("a wrong current password is named under its field and nothing changes", async () => {
		const view = await mountDevices(<Harness />);
		const before = await vaultBytes(view);
		await fill("not the current password", NEW_PASSWORD);
		await view.settle();
		expect(byRole("dialog").textContent).toContain(
			"That password doesn't open the keys for edge-berlin-01 on this computer.",
		);
		expect(typed()).toEqual(["", "", ""]);
		expect(await vaultBytes(view)).toBe(before);
	});

	test("the new password must be 12 bytes or more and differ from the current one", async () => {
		const view = await mountDevices(<Harness />);
		await fill(view.fake.password, "short");
		expect(byRole("dialog").textContent).toContain(
			"The new password is 5 bytes. Use at least 12.",
		);
		await fill(view.fake.password, view.fake.password);
		expect(byRole("dialog").textContent).toContain(
			"Choose a different password from the current one.",
		);
		expect(view.fake.workspace.keys.snapshot(EDGE).state).toBe("unlocked");
	});
});
