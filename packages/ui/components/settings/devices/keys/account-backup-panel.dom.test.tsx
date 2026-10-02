import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type { BackupMode } from "./account-backup-panel";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { AccountBackupPanel, AccountBackupSheet, RestoreKeysSheet } =
	await import("./account-backup-panel");
const { rowScope, useKeyResults } = await import("./key-store");
const { useKeysModel } = await import("./use-keys-model");
const { BackupsStamp } = await import("./keys-table");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const VAULTS = /^devices\/controller-vaults\//;
const NO_FLOWS = {
	open() {},
	retryUpload() {},
	deleteKeys() {},
	unlockSeveral() {},
	busy: () => false,
};

/** One device's backup sheet over the real model, with its row result next to it. */
function BackupHarness({
	deviceId,
	mode = "save",
	show = true,
}: Readonly<{ deviceId: string; mode?: BackupMode; show?: boolean }>) {
	const read = useKeysModel();
	const results = useKeyResults();
	const row = read.model.rows.find((entry) => entry.deviceId === deviceId);
	return (
		<>
			{show && row ? (
				<AccountBackupSheet row={row} mode={mode} onClose={() => undefined} />
			) : null}
			<output data-result="">{results.of(rowScope(deviceId))?.text}</output>
		</>
	);
}

function RestoreHarness({ show = true }: Readonly<{ show?: boolean }>) {
	const read = useKeysModel();
	return show && read.devices.loaded ? (
		<RestoreKeysSheet
			read={read}
			preselect={SAMPLE_IDS.edge}
			flows={NO_FLOWS}
			onClose={() => undefined}
		/>
	) : null;
}

function PanelHarness() {
	const read = useKeysModel();
	return (
		<>
			<AccountBackupPanel read={read} />
			<BackupsStamp read={read} />
		</>
	);
}

function newComputer(): MountDevicesOptions {
	const seed = sampleFleet();
	seed.local = { ...seed.local, vaults: [], backups: {} };
	seed.keys = [];
	return { seed, unlock: "none" };
}

type View = Awaited<ReturnType<typeof mountDevices>>;

const field = () => byRole("dialog").querySelector("input") as HTMLInputElement;
const resultOf = (view: View) =>
	view.container.querySelector("[data-result]")?.textContent ?? "";
const puts = (view: View) => view.fake.api.sent("PUT", VAULTS);
const vaultIds = (view: View) =>
	view.fake.workspace.local.summary().vaults.map((vault) => vault.deviceId);

async function submitBackup(view: View, password = view.fake.password) {
	await typeInto(field(), password);
	await click(byRole("button", "Back up to account", byRole("dialog")));
}

async function submitRestore(view: View, password = view.fake.password) {
	const row = byRole("dialog").querySelector(
		`[data-restore-row="${SAMPLE_IDS.edge}"] input[type="password"]`,
	) as HTMLInputElement;
	await typeInto(row, password);
	await click(byRole("button", /^Restore 1 device/, byRole("dialog")));
}

describe("account backup sheet (inherits device-account-recovery 1–8)", () => {
	test("saving clears the password at once, blocks a second submit and reports the saved version", async () => {
		const view = await mountDevices(
			<BackupHarness deviceId={SAMPLE_IDS.cold} />,
		);
		const release = view.fake.api.hold({ method: "PUT", path: VAULTS });
		await submitBackup(view);
		await view.settle();
		expect(field().value).toBe("");
		expect(field().disabled).toBe(true);
		expect(puts(view).length).toBe(1);
		await click(byRole("button", "Back up to account", byRole("dialog")));
		expect(puts(view).length).toBe(1);
		expect(resultOf(view)).toBe("");

		release();
		await view.settle();
		expect(resultOf(view)).toContain("Backed up to your account as version 1");
		expect(JSON.stringify(view.fake.api.calls)).not.toContain(
			view.fake.password,
		);
	});

	test("restoring clears the passwords at once and reports only the completed result", async () => {
		const view = await mountDevices(<RestoreHarness />, newComputer());
		const release = view.fake.api.hold({ method: "GET", path: VAULTS });
		await submitRestore(view);
		await view.settle();
		expect(byRole("dialog").querySelector('input[type="password"]')).toBeNull();
		expect(vaultIds(view)).toEqual([]);
		expect(byRole("dialog").textContent).not.toContain("Restored keys for");

		release();
		await view.settle();
		expect(vaultIds(view)).toEqual([SAMPLE_IDS.edge]);
		expect(byRole("dialog").textContent).toContain("Restored v3 · locked");
		expect(byRole("dialog").textContent).toContain(
			"Restored keys for edge-berlin-01",
		);
	});

	test("a failed save stays visible and keeps no password", async () => {
		const view = await mountDevices(
			<BackupHarness deviceId={SAMPLE_IDS.cold} />,
		);
		view.fake.api.fail({ method: "PUT", path: VAULTS });
		await submitBackup(view);
		await view.settle();
		expect(byRole("dialog").textContent).toContain(
			"The hub reported an error. Nothing was changed on this computer; try again in a moment.",
		);
		expect(field().value).toBe("");
		expect(field().disabled).toBe(false);
		expect(resultOf(view)).toBe("");
	});

	test("a failed restore stays visible and keeps no password", async () => {
		const view = await mountDevices(<RestoreHarness />, newComputer());
		await submitRestore(view, "not-the-right-password");
		await view.settle();
		const dialog = byRole("dialog");
		expect(dialog.textContent).toContain(
			"That password doesn't open this backup, or the backup was changed.",
		);
		expect(dialog.textContent).toContain("Nothing was restored");
		expect(dialog.querySelector('input[type="password"]')).toBeNull();
		expect(vaultIds(view)).toEqual([]);
		expect(dialog.textContent).not.toContain("not-the-right-password");
	});

	for (const completion of ["success", "failure"] as const) {
		test(`another device's form ignores the stale ${completion} of the one before`, async () => {
			const view = await mountDevices(
				<BackupHarness deviceId={SAMPLE_IDS.cold} />,
			);
			const release = view.fake.api.hold({ method: "PUT", path: VAULTS });
			await submitBackup(view);
			await view.settle();
			await view.rerender(
				<BackupHarness deviceId={SAMPLE_IDS.edge} mode="update" />,
			);
			expect(field().value).toBe("");
			expect(field().disabled).toBe(false);
			await typeInto(field(), "typed for the next one");

			if (completion === "failure")
				view.fake.api.fail({ method: "PUT", path: VAULTS }, undefined, 1);
			release();
			await view.settle();
			expect(field().disabled).toBe(false);
			expect(field().value).toBe("typed for the next one");
			expect(byRole("dialog").textContent).not.toContain("reported an error");
			expect(resultOf(view)).toBe("");
			expect(queryByRole("alert")).toBeNull();
		});
	}

	test("closing the sheet aborts a restore; a late answer restores nothing", async () => {
		const view = await mountDevices(<RestoreHarness />, newComputer());
		const release = view.fake.api.hold({ method: "GET", path: VAULTS });
		await submitRestore(view);
		await view.settle();
		await view.rerender(<RestoreHarness show={false} />);
		release();
		await view.settle();
		expect(vaultIds(view)).toEqual([]);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("closing the sheet aborts a pending backup; its late failure reaches nobody", async () => {
		const view = await mountDevices(
			<BackupHarness deviceId={SAMPLE_IDS.cold} />,
		);
		const release = view.fake.api.hold({ method: "PUT", path: VAULTS });
		await submitBackup(view);
		await view.settle();
		await view.rerender(
			<BackupHarness deviceId={SAMPLE_IDS.cold} show={false} />,
		);
		release();
		await view.settle();
		expect(view.fake.hub.backups.has(SAMPLE_IDS.cold)).toBe(false);
		expect(resultOf(view)).toBe("");
		expect(queryByRole("alert")).toBeNull();
	});
});

describe("account backup sheet: checking", () => {
	test("Check account backup opens it with the password and adopts the account's version", async () => {
		const view = await mountDevices(
			<BackupHarness deviceId={SAMPLE_IDS.edge} mode="check" />,
		);
		expect(byRole("dialog").textContent).toContain(
			"This computer v3 · your account v3",
		);
		const before = view.fake.api.writes().length;
		await typeInto(field(), view.fake.password);
		await click(byRole("button", "Check backup", byRole("dialog")));
		await view.settle();
		expect(resultOf(view)).toContain(
			"your account's backup v3 opens with that password",
		);
		expect(view.fake.api.writes().length).toBe(before);
	});
});

describe("Your account block", () => {
	test("shows the slots and compares versions without sending anything", async () => {
		const view = await mountDevices(<PanelHarness />);
		expect(view.container.textContent).toContain("4 of 256 slots");
		const before = view.fake.api.writes().length;
		await click(byRole("button", "Check backups"));
		await view.settle();
		expect(view.container.textContent).toContain("Checked 4 account backups");
		expect(view.fake.api.writes().length).toBe(before);
	});

	test("a failed check keeps the versions and says from when they are", async () => {
		const view = await mountDevices(<PanelHarness />);
		view.fake.api.fail(
			{ method: "GET", path: "devices/controller-vaults" },
			Object.assign(new Error("Forbidden"), { status: 403, code: "FORBIDDEN" }),
		);
		await click(byRole("button", "Check backups"));
		await view.settle();
		expect(view.container.textContent).toContain(
			"Couldn't reach your account at",
		);
		expect(view.container.textContent).toContain("The versions shown are from");
		expect(view.container.textContent).toContain("4 of 256 slots");
		expect(
			view.container
				.querySelector('[data-stamp][data-src="hub"]')
				?.getAttribute("data-age"),
		).toBe("error");
	});

	test("an older hub shows the interim instead of slots and no error", async () => {
		const view = await mountDevices(<PanelHarness />, { hubVersion: "old" });
		expect(view.container.textContent).toContain(
			"4 for the devices in your list",
		);
		expect(view.container.textContent).toContain(
			"This hub reports backups one device at a time",
		);
		expect(view.container.textContent).not.toContain("of 256 slots");
		expect(queryByRole("alert")).toBeNull();
		expect(
			view.fake.api.sent("GET", "devices/controller-vaults").length,
		).toBeLessThanOrEqual(1);
	});
});
