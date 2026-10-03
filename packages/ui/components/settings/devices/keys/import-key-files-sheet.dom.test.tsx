import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	byRole,
	click,
	dropFiles,
	installDom,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { sealedControllerBackup } = await import(
	"../../../../lib/device-management/recovery"
);
const { deleteDeviceVault, readDeviceVault } = await import(
	"../../../../lib/device-management/storage"
);
await preloadDevices();
const { ImportKeyFilesSheet } = await import("./import-key-files-sheet");
const { keyFileName } = await import("./key-operations");
const { useKeysModel } = await import("./use-keys-model");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge: EDGE, warehouse: WAREHOUSE } = SAMPLE_IDS;

function Harness() {
	const read = useKeysModel();
	return read.devices.loaded ? (
		<ImportKeyFilesSheet read={read} onClose={() => undefined} />
	) : null;
}

/** The sealed backup file of a device's keys as this computer holds them. */
async function backupFile(fake: FakeWorkspace, deviceId: string) {
	const vault = await readDeviceVault(fake.scope, deviceId);
	if (!vault) throw new Error(`The seed has no keys for ${deviceId}`);
	const blob = await sealedControllerBackup(
		fake.scope,
		vault,
		fake.password,
		fake.crypto,
	);
	return new File([await blob.text()], keyFileName(deviceId), {
		type: "application/json",
	});
}

async function forget(fake: FakeWorkspace, ...deviceIds: string[]) {
	for (const deviceId of deviceIds)
		await deleteDeviceVault(fake.scope, deviceId);
	await fake.workspace.local.reload();
}

function vaultIds(fake: FakeWorkspace): string[] {
	const idOf = (vault: { deviceId: string }) => vault.deviceId;
	return fake.workspace.local.summary().vaults.map(idOf);
}

function rowOf(name: string): HTMLElement {
	const row = byRole("dialog").querySelector(`[data-import-file="${name}"]`);
	if (!row) throw new Error(`No import row for ${name}`);
	return row as HTMLElement;
}

function passwordOf(name: string): HTMLInputElement {
	const field = rowOf(name).querySelector("input");
	if (!field) throw new Error(`No password field for ${name}`);
	return field;
}

async function drop(files: File[]): Promise<void> {
	const zone = byRole("dialog").querySelector("label") as HTMLElement;
	await dropFiles(zone, files);
}

/** A computer without the two devices' keys, with four files dropped: two usable, two not. */
async function fourFiles() {
	const fake = await createFakeWorkspace(undefined, { unlock: "none" });
	const edge = await backupFile(fake, EDGE);
	const warehouse = await backupFile(fake, WAREHOUSE);
	const elsewhere = new File(
		[(await edge.text()).replace(fake.scope.apiOrigin, "https://other.test")],
		keyFileName("11111111-2222-4333-8444-555555555555"),
	);
	const junk = new File(["not json at all"], "holiday.json");
	await forget(fake, EDGE, WAREHOUSE);
	const view = await mountDevices(<Harness />, { fake });
	await drop([edge, warehouse, elsewhere, junk]);
	await view.settle();
	return { fake, view, edge, warehouse, elsewhere };
}

describe("Import key backup files", () => {
	test("several files at once: bad files stay listed with the reason, sealed files ask for their password", async () => {
		const { fake, edge, elsewhere } = await fourFiles();
		expect(rowOf(edge.name).textContent).toContain("Owner keys for");
		expect(rowOf(edge.name).textContent).toContain("edge-berlin-01");
		expect(rowOf(elsewhere.name).textContent).toContain(
			"This backup belongs to another device or hub.",
		);
		expect(rowOf("holiday.json").textContent).toContain(
			"This isn't a key backup file.",
		);
		await click(byRole("button", /^Verify and import 2 files/));
		expect(byRole("dialog").textContent).toContain(
			"Enter the device password for edge-berlin-01 and warehouse-pi.",
		);
		expect(vaultIds(fake)).not.toContain(EDGE);
	});

	test("each sealed file takes its own password: the right one imports, the wrong one is named", async () => {
		const { fake, view, edge, warehouse } = await fourFiles();
		await typeInto(passwordOf(edge.name), fake.password);
		await typeInto(passwordOf(warehouse.name), "definitely the wrong one");
		await click(byRole("button", /^Verify and import 2 files/));
		await view.settle();

		expect(vaultIds(fake)).toContain(EDGE);
		expect(vaultIds(fake)).not.toContain(WAREHOUSE);
		expect(fake.workspace.keys.snapshot(EDGE).state).toBe("locked");
		expect(rowOf(edge.name).textContent).toContain(
			"Imported for edge-berlin-01 · locked",
		);
		expect(rowOf(warehouse.name).textContent).toContain(
			"That password doesn't open this backup",
		);
		const dialog = byRole("dialog");
		expect(dialog.textContent).toContain("Imported keys for edge-berlin-01.");
		expect(dialog.textContent).toContain("Not imported: warehouse-pi.");
		expect(dialog.textContent).toContain("2 files couldn't be used.");
		expect(dialog.querySelectorAll("input").length).toBe(0);
		expect(dialog.textContent).not.toContain("definitely the wrong one");
	});

	test("a file of keys that are already here changes nothing and says so", async () => {
		const fake = await createFakeWorkspace(undefined, { unlock: "none" });
		const edge = await backupFile(fake, EDGE);
		const view = await mountDevices(<Harness />, { fake });
		const before = vaultIds(fake).length;
		await drop([edge]);
		await view.settle();
		await typeInto(passwordOf(edge.name), fake.password);
		await click(byRole("button", /^Verify and import 1 file/));
		await view.settle();
		expect(rowOf(edge.name).textContent).toContain(
			"edge-berlin-01 already here · nothing changed",
		);
		expect(vaultIds(fake).length).toBe(before);
	});
});
