import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../lib/api-error";
import {
	SAMPLE_IDS,
	SAMPLE_PEOPLE,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { KeysRoute } from "../../../../lib/device-management/model/types";
import {
	advance,
	allByRole,
	byRole,
	byText,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { KeysScreen } = await import("./keys-screen");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const ACCOUNT = { kind: "account" } as const;
const HOME: KeysRoute = { screen: "keys" };
/** R3: wire values, condition keys and gate codes never reach the screen. */
const MACHINE_WORDS =
	/[a-z]+_[a-z_]+|\bG\d{1,2}\b|\bD\d{1,2}\b|controller|vault|MLS|endpoint/;

function mount(options: MountDevicesOptions = {}, route: KeysRoute = HOME) {
	return mountDevices(<KeysScreen route={route} scope={ACCOUNT} />, options);
}

function rowOf(container: HTMLElement, deviceId: string): HTMLElement {
	const row = container.querySelector(`tr[data-device="${deviceId}"]`);
	if (!row) throw new Error(`No keys row for ${deviceId}`);
	return row as HTMLElement;
}

function windowOf(container: HTMLElement, name: string): HTMLElement {
	const node = container.querySelector(`[data-window="${name}"]`);
	if (!node) throw new Error(`No summary window ${name}`);
	return node as HTMLElement;
}

type View = Awaited<ReturnType<typeof mount>>;

function puts(view: View) {
	return view.fake.api.sent("PUT", /^devices\/controller-vaults\//);
}

/** Hub writes of the key flows; the workspace's own status subscriptions are not ours. */
function keyWrites(view: View) {
	const ours = ([, path]: [string, string, unknown?]) =>
		path.includes("controller-vaults");
	return view.fake.api.writes().filter(ours);
}

function vaultIds(view: View): string[] {
	const idOf = (vault: { deviceId: string }) => vault.deviceId;
	return view.fake.workspace.local.summary().vaults.map(idOf);
}

function hasVault(view: View, deviceId: string): boolean {
	return vaultIds(view).includes(deviceId);
}

function newComputer(): MountDevicesOptions {
	const seed = sampleFleet();
	seed.local = { ...seed.local, vaults: [], backups: {} };
	seed.keys = [];
	return { seed, unlock: "none" };
}

function restoreRow(deviceId: string): HTMLElement {
	const row = byRole("dialog").querySelector(
		`[data-restore-row="${deviceId}"]`,
	);
	if (!row) throw new Error(`No restore row for ${deviceId}`);
	return row as HTMLElement;
}

function passwordOf(row: HTMLElement): HTMLInputElement {
	const field = row.querySelector('input[type="password"]');
	if (!field) throw new Error("The restore row has no password field");
	return field as HTMLInputElement;
}

function isTicked(box: HTMLElement): boolean {
	return (
		box.getAttribute("aria-checked") === "true" && !box.hasAttribute("disabled")
	);
}

/** Leaves one device ticked in the restore sheet and returns its row. */
async function restoreOnly(deviceId: string): Promise<HTMLElement> {
	const keep = restoreRow(deviceId);
	for (const box of allByRole("checkbox", undefined, byRole("dialog"))) {
		const row = box.closest("[data-restore-row]");
		if (row && row !== keep && isTicked(box)) await click(box);
	}
	return restoreRow(deviceId);
}

describe("Keys & recovery: the golden fleet", () => {
	test("says in one headline which keys are at risk, and sends nothing", async () => {
		const view = await mount();
		const { container } = view;
		expect(byRole("heading", "Keys & recovery")).toBeTruthy();
		const headline = container.querySelector("[data-headline]");
		expect(headline?.textContent).toContain(
			"This computer holds keys for 6 devices.",
		);
		expect(headline?.textContent).toContain(
			"cold-storage-nas's keys exist only here.",
		);
		expect(headline?.textContent).toContain(
			"studio-mac-mini's latest backup hasn't reached your account.",
		);
		expect(container.textContent).toContain("4 of 256 slots");
		expect(container.textContent).toContain("Desktop app · kept safely");
		expect(
			container.querySelectorAll("[data-dv-primary]").length,
		).toBeLessThanOrEqual(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
		expect(keyWrites(view)).toEqual([]);
	});

	test("lists every device with its keys, account backup and what to do next", async () => {
		const { container } = await mount();
		expect(rowOf(container, SAMPLE_IDS.edge).dataset.category).toBe("synced");
		expect(rowOf(container, SAMPLE_IDS.edge).textContent).toContain(
			"Backed up (v3)",
		);
		expect(rowOf(container, SAMPLE_IDS.warehouse).dataset.category).toBe(
			"synced",
		);
		const studio = rowOf(container, SAMPLE_IDS.studio);
		expect(studio.dataset.category).toBe("pending");
		expect(studio.textContent).toContain("This computer v2");
		expect(studio.textContent).toContain("account v1");
		expect(studio.textContent).toContain("Retry upload");
		const cold = rowOf(container, SAMPLE_IDS.cold);
		expect(cold.dataset.category).toBe("never");
		expect(cold.textContent).toContain("Back up to account");
		const lab = rowOf(container, SAMPLE_IDS.lab);
		expect(lab.textContent).toContain("Shared-access keys");
		const partner = rowOf(container, SAMPLE_IDS.partner);
		expect(partner.dataset.category).toBe("nokeys");
		expect(partner.textContent).toContain("Cloud approvals only");
		expect(partner.textContent).toContain("revoked");

		expect(windowOf(container, "here").textContent).toContain("6");
		expect(windowOf(container, "synced").textContent).toContain("3");
		expect(windowOf(container, "behind").textContent).toContain(
			"Upload pending",
		);
		expect(windowOf(container, "never").textContent).toContain(
			"cold-storage-nas · only here",
		);

		const local = container.querySelector("#local-only");
		expect(local?.textContent).toContain("old-kiosk");
		expect(local?.textContent).toContain("Revoked.");
		expect(
			container.querySelector('[data-local-only="request"]')?.textContent,
		).toContain("isn't approved yet");
	});

	test("names who shared a device once the account directory answers", async () => {
		const unnamed = await mount();
		expect(rowOf(unnamed.container, SAMPLE_IDS.lab).textContent).toContain(
			"Shared with you",
		);
		await cleanupDevices();

		const fake = await createFakeWorkspace(sampleFleet());
		const mira = { id: SAMPLE_PEOPLE.mira, name: "Mira Novak" };
		const userState = {
			getProfile: async () => fake.profile,
			getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
			lookupUsers: async (ids: string[]) =>
				ids.includes(mira.id) ? [mira] : [],
		};
		const named = await mount({
			fake,
			backend: { userState: userState as never },
		});
		await advance(40);
		await named.settle();
		expect(rowOf(named.container, SAMPLE_IDS.lab).textContent).toContain(
			"Shared by Mira Novak",
		);
	});

	test("a summary window filters the table and the chip clears it", async () => {
		const { container } = await mount();
		await click(windowOf(container, "never"));
		const rows = Array.from(container.querySelectorAll("tr[data-device]"));
		expect(rows.map((row) => (row as HTMLElement).dataset.device)).toEqual([
			SAMPLE_IDS.cold,
		]);
		expect(container.textContent).toContain("Showing 1 of 6");
		await click(
			byRole(
				"button",
				/Not backed up/,
				container.querySelector("#keys-table") as HTMLElement,
			),
		);
		expect(container.querySelectorAll("tr[data-device]").length).toBe(6);
	});

	test("focus=<device> selects the row; a guide link opens its steps", async () => {
		const { container } = await mount(
			{},
			{
				screen: "keys",
				focusDeviceId: SAMPLE_IDS.cold,
				guide: "forgot-password",
			},
		);
		expect(
			rowOf(container, SAMPLE_IDS.cold).getAttribute("aria-selected"),
		).toBe("true");
		const guide = container.querySelector("#guide-forgot-password");
		expect(guide?.textContent).toContain("Still works without it");
		expect(guide?.textContent).toContain("Revoking the device");
		expect(guide?.textContent).toContain("You own cold-storage-nas");
		expect(guide?.textContent).not.toContain("It was shared with you");
	});
});

describe("Keys & recovery: account backups", () => {
	test("Retry upload publishes the staged backup without asking for a password", async () => {
		const view = await mount();
		const { container } = view;
		await click(
			byRole("button", "Retry upload", rowOf(container, SAMPLE_IDS.studio)),
		);
		await view.settle();
		expect(puts(view).length).toBe(1);
		expect(queryByRole("dialog")).toBeNull();
		expect(container.textContent).toContain(
			"The backup reached your account as version 2",
		);
		expect(rowOf(container, SAMPLE_IDS.studio).dataset.category).toBe("synced");
	});

	test("Back up to account seals with the typed password, clears it and reports the version", async () => {
		const view = await mount();
		const { container } = view;
		await click(
			byRole("button", "Back up to account", rowOf(container, SAMPLE_IDS.cold)),
		);
		const sheet = byRole("dialog");
		const field = sheet.querySelector("input") as HTMLInputElement;
		const submit = byRole("button", "Back up to account", sheet);
		await click(submit);
		expect(puts(view)).toEqual([]);

		await typeInto(field, "not-the-password");
		await click(submit);
		await view.settle();
		expect(puts(view)).toEqual([]);
		expect(byRole("dialog").textContent).toContain(
			"That password doesn't open the keys for cold-storage-nas on this computer.",
		);
		expect(
			(byRole("dialog").querySelector("input") as HTMLInputElement).value,
		).toBe("");

		await typeInto(
			byRole("dialog").querySelector("input") as HTMLInputElement,
			view.fake.password,
		);
		await click(byRole("button", "Back up to account", byRole("dialog")));
		await view.settle();
		expect(puts(view).length).toBe(1);
		expect(queryByRole("dialog")).toBeNull();
		expect(container.textContent).toContain(
			"Backed up to your account as version 1",
		);
		expect(JSON.stringify(view.fake.api.calls)).not.toContain(
			view.fake.password,
		);
		expect(rowOf(container, SAMPLE_IDS.cold).dataset.category).toBe("synced");
	});

	test("Check backups compares versions only and says what differs", async () => {
		const view = await mount();
		await click(byRole("button", "Check backups"));
		await view.settle();
		expect(view.container.textContent).toContain("Checked 4 account backups");
		expect(view.container.textContent).toContain("3 match this computer");
		expect(view.container.textContent).toContain(
			"studio-mac-mini differs from this computer",
		);
		expect(view.container.textContent).toContain("cold-storage-nas has none");
		expect(keyWrites(view)).toEqual([]);
	});
});

describe("Keys & recovery: older hub and older agent", () => {
	test("a hub without the backup list reads one device at a time and shows the interim", async () => {
		const view = await mount({ hubVersion: "old" });
		const { container } = view;
		expect(container.textContent).toContain(
			"This hub reports backups one device at a time",
		);
		expect(container.textContent).not.toContain("of 256 slots");
		expect(rowOf(container, SAMPLE_IDS.edge).textContent).toContain(
			"Backed up (v3)",
		);
		expect(rowOf(container, SAMPLE_IDS.cold).dataset.category).toBe("never");
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(
			view.fake.api.sent("GET", "devices/controller-vaults").length,
		).toBeLessThanOrEqual(1);
		for (const id of [SAMPLE_IDS.edge, SAMPLE_IDS.cold, SAMPLE_IDS.lab])
			expect(
				view.fake.api.sent("GET", `devices/controller-vaults/${id}`).length,
			).toBeLessThanOrEqual(1);
		expect(keyWrites(view)).toEqual([]);
	});

	test("an agent without the features map changes nothing here: no device command is sent", async () => {
		const view = await mount({ agentFeatures: {}, unlock: "none" });
		const before = view.fake.api.commands.length;
		expect(rowOf(view.container, SAMPLE_IDS.edge).textContent).toContain(
			"Backed up (v3)",
		);
		await click(byRole("button", "Check backups"));
		await view.settle();
		expect(view.fake.api.commands.length).toBe(before);
		expect(view.container.querySelector('[role="alert"]')).toBeNull();
	});
});

describe("Keys & recovery: storage and recovery states", () => {
	test("a browser that may delete keys warns and offers Keep keys safely as the one primary action", async () => {
		const view = await mount({ platform: "web", persistence: "denied" });
		const { container } = view;
		expect(container.textContent).toContain(
			"This site hasn't been granted persistent storage.",
		);
		expect(container.textContent).toContain(
			"This browser holds keys for 6 devices, and may delete them.",
		);
		const primary = Array.from(container.querySelectorAll("[data-dv-primary]"));
		expect(primary.length).toBe(1);
		expect(primary[0]?.textContent).toContain("Keep keys safely");
		view.fake.browser.persistence = "persisted";
		await click(primary[0] as HTMLElement);
		await view.settle();
		expect(container.textContent).toContain(
			"The browser granted persistent storage",
		);
	});

	test("a new computer leads with the restore guide as its one primary action", async () => {
		const { container } = await mount(newComputer());
		expect(container.textContent).toContain(
			"This computer has no device keys yet.",
		);
		const guide = container.querySelector("#guide-new-computer") as HTMLElement;
		expect(guide.textContent).toContain(
			"Start here: bring your keys to this computer",
		);
		const primary = container.querySelectorAll("[data-dv-primary]");
		expect(primary.length).toBe(1);
		expect(rowOf(container, SAMPLE_IDS.edge).dataset.category).toBe(
			"restorable",
		);
		expect(rowOf(container, SAMPLE_IDS.cold).dataset.category).toBe("lost");
		await click(byRole("button", "Restore keys…", guide));
		expect(byRole("dialog").textContent).toContain(
			"Restore keys from your account",
		);
	});

	test("a new computer restores one device with its backup's password, never a wrong one", async () => {
		const view = await mount(newComputer());
		await click(byRole("button", "Restore keys…"));
		const edge = await restoreOnly(SAMPLE_IDS.edge);
		await typeInto(passwordOf(edge), "wrong-password-xx");
		await click(byRole("button", /^Restore 1 device/, byRole("dialog")));
		await view.settle();
		expect(byRole("dialog").textContent).toContain(
			"That password doesn't open this backup",
		);
		expect(vaultIds(view)).toEqual([]);

		await click(byRole("button", "Try other passwords", byRole("dialog")));
		const again = passwordOf(restoreRow(SAMPLE_IDS.edge));
		expect(again.value).toBe("");
		await typeInto(again, view.fake.password);
		await click(byRole("button", /^Restore 1 device/, byRole("dialog")));
		await view.settle();
		expect(byRole("dialog").textContent).toContain("Restored v3 · locked");
		expect(vaultIds(view)).toEqual([SAMPLE_IDS.edge]);
		expect(view.fake.workspace.keys.snapshot(SAMPLE_IDS.edge).state).toBe(
			"locked",
		);
	});
});

describe("Keys & recovery: before the device list is there", () => {
	test("a device list that can't be loaded never reads as no keys, no devices or nothing unlocked", async () => {
		const fake = await createFakeWorkspace();
		// An answer the client can't use is a verdict at once (no retries to wait for).
		fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({ status: 400, code: "BAD_REQUEST", message: "" }),
		);
		const view = await mount({ fake });
		const { container } = view;
		const headline = container.querySelector("[data-headline]");
		expect(headline?.textContent).toContain(
			"This computer holds keys for 7 devices.",
		);
		expect(headline?.textContent).toContain(
			"Your device list couldn't be loaded",
		);
		expect(container.textContent).not.toContain("no device keys yet");
		expect(container.textContent).not.toContain("0 devices in your list");
		expect(container.querySelector("[data-window]")).toBeNull();
		expect(container.textContent).toContain(
			"Couldn't load your device list from the hub.",
		);
		expect(container.textContent).not.toContain(
			"No keys on this computer yet.",
		);
		for (const name of [
			"Unlock several…",
			"Download backup files…",
			"Change device password…",
		])
			expect(
				byRole("button", name, container).getAttribute("aria-disabled"),
			).toBe("true");
		expect(container.textContent).toContain(
			"Available once your device list has loaded.",
		);
		// Locking needs no list: the open key sessions are this computer's own fact.
		expect(container.textContent).toContain("3 devices");
		await click(byRole("button", "Lock all", container));
		expect(
			view.fake.workspace.keys.list().filter((row) => row.state === "unlocked"),
		).toEqual([]);
		expect(keyWrites(view)).toEqual([]);
	});

	test("Restore keys… waits for the device list instead of saying every device has keys", async () => {
		const fake = await createFakeWorkspace();
		fake.api.fail(
			{ method: "GET", path: "devices" },
			new ApiResponseError({ status: 400, code: "BAD_REQUEST", message: "" }),
		);
		const view = await mount({ fake });
		await click(byRole("button", "Restore keys…", view.container));
		const sheet = byRole("dialog");
		expect(sheet.textContent).toContain(
			"Couldn't load your device list from the hub.",
		);
		expect(sheet.textContent).not.toContain("already has keys here");
		expect(queryByRole("checkbox", undefined, sheet)).toBeNull();
		await click(byRole("button", "Close", sheet));
		expect(queryByRole("dialog")).toBeNull();
	});
});

describe("Keys & recovery: deleting keys", () => {
	test("keys without any backup need the device name typed; cancel sends nothing", async () => {
		const view = await mount();
		const { container } = view;
		await click(byRole("button", /More for cold-storage-nas/, container));
		await click(byText("Delete keys from this computer…", document.body));
		const sheet = byRole("alertdialog");
		expect(sheet.textContent).toContain("not backed up · the only copy");
		expect(sheet.textContent).toContain("No, this is permanent.");
		const confirm = byRole("button", "Delete keys for cold-storage-nas", sheet);
		await click(confirm);
		expect(hasVault(view, SAMPLE_IDS.cold)).toBe(true);
		await typeInto(
			sheet.querySelector("input") as HTMLInputElement,
			"cold-storage-nas",
		);
		await click(byRole("button", "Delete keys for cold-storage-nas", sheet));
		await view.settle();
		expect(hasVault(view, SAMPLE_IDS.cold)).toBe(false);
		expect(container.textContent).toContain(
			"No copy is left here or on your account",
		);
	});

	test("a first upload that never reached the account still counts as the only copy: the name is typed", async () => {
		const seed = sampleFleet();
		seed.local = {
			...seed.local,
			backups: {
				...seed.local.backups,
				[SAMPLE_IDS.cold]: { localRevision: 1, pending: true },
			},
		};
		const view = await mount({ seed });
		const { container } = view;
		expect(rowOf(container, SAMPLE_IDS.cold).dataset.category).toBe("pending");
		await click(byRole("button", /More for cold-storage-nas/, container));
		await click(byText("Delete keys from this computer…", document.body));
		const sheet = byRole("alertdialog");
		expect(sheet.textContent).toContain("not backed up · the only copy");
		expect(sheet.textContent).toContain("No, this is permanent.");
		expect(sheet.textContent).toContain("Retry the upload first");
		expect(sheet.textContent).not.toContain("v0");
		expect(queryByRole("checkbox", undefined, sheet)).toBeNull();
		await click(byRole("button", "Delete keys for cold-storage-nas", sheet));
		expect(hasVault(view, SAMPLE_IDS.cold)).toBe(true);
	});

	test("keys of a revoked device are deleted from Local-only keys after one confirm", async () => {
		const view = await mount();
		const { container } = view;
		const local = container.querySelector("#local-only") as HTMLElement;
		const row = local.querySelector(
			'[data-local-only="revoked"]',
		) as HTMLElement;
		await click(byRole("button", "Delete keys…", row));
		const sheet = byRole("alertdialog");
		expect(sheet.textContent).toContain("device revoked");
		expect(sheet.textContent).toContain("These keys can't be used any more.");
		await click(byRole("button", "Delete keys for old-kiosk", sheet));
		await view.settle();
		expect(hasVault(view, SAMPLE_IDS.oldKiosk)).toBe(false);
		expect(container.textContent).toContain(
			"Keys for old-kiosk were deleted from this computer",
		);
	});
});

describe("Keys & recovery: this computer", () => {
	test("asking for the password again for access changes is a switch, off by default", async () => {
		const view = await mount();
		const toggle = byRole(
			"switch",
			"Ask for my password again for access changes",
		);
		expect(toggle.getAttribute("aria-checked")).toBe("false");
		await click(toggle);
		expect(view.fake.workspace.keys.askPasswordForAccessChanges()).toBe(true);
		expect(
			byRole(
				"switch",
				"Ask for my password again for access changes",
			).getAttribute("aria-checked"),
		).toBe("true");
	});

	test("Lock all closes every key session; Unlock several is offered again", async () => {
		const view = await mount();
		const { container } = view;
		expect(container.textContent).toContain("3 devices");
		await click(byRole("button", "Lock all", container));
		await view.settle();
		expect(
			view.fake.workspace.keys
				.list()
				.filter((session) => session.state === "unlocked"),
		).toEqual([]);
		expect(
			byRole("button", "Lock all", container).getAttribute("aria-disabled"),
		).toBe("true");
		expect(container.textContent).toContain("Nothing is unlocked right now.");
		expect(
			byRole("button", "Unlock several…", container).getAttribute(
				"aria-disabled",
			),
		).toBeNull();
	});

	test("the key actions of the table say why they are off while no keys are here", async () => {
		const view = await mount(newComputer());
		const table = view.container.querySelector("#keys-table") as HTMLElement;
		const download = byRole("button", "Download backup files…", table);
		expect(download.getAttribute("aria-disabled")).toBe("true");
		expect(table.textContent).toContain("No keys on this computer yet.");
		const calls = view.fake.api.calls.length;
		await click(download);
		await click(byRole("button", "Change device password…", table));
		expect(queryByRole("dialog")).toBeNull();
		expect(view.fake.api.calls.length).toBe(calls);
	});

	test("the row menu follows the same gates as the buttons: an action that can't run is off and says why", async () => {
		const view = await mount();
		const { container, fake } = view;
		const menuItem = (id: string) =>
			document.body.querySelector(`[data-menu-item="${id}"]`) as HTMLElement;
		await click(byRole("button", /More for cold-storage-nas/, container));
		expect(menuItem("backup").getAttribute("aria-disabled")).toBeNull();
		await click(menuItem("backup"));
		expect(byRole("dialog").textContent).toContain("Back up to account");
		await click(byRole("button", "Cancel", byRole("dialog")));

		// Account backups need the hub; the files on this computer don't.
		fake.api.mode.tokenRestricted = true;
		await act(async () => {
			await fake.queryClient.refetchQueries();
		});
		await view.settle();
		const reason =
			"Your access token is restricted. Use a token with full permissions.";
		expect(rowOf(container, SAMPLE_IDS.cold).textContent).toContain(reason);
		await click(byRole("button", /More for cold-storage-nas/, container));
		for (const id of ["backup", "check"]) {
			expect(menuItem(id).getAttribute("aria-disabled")).toBe("true");
			expect(menuItem(id).textContent).toContain(reason);
		}
		for (const id of ["download", "password", "delete"])
			expect(menuItem(id).getAttribute("aria-disabled")).toBeNull();
		await click(menuItem("backup"));
		expect(queryByRole("dialog")).toBeNull();
		expect(keyWrites(view)).toEqual([]);
	});
});
