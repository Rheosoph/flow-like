import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_PEOPLE,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	byRole,
	click,
	clickByText,
	dropFiles,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountedDevices } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { MACHINE_WORDS, mountAccess } = await import("./access-test-kit");
const { AccessScreen } = await import("./access-screen");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { fakeCompact, fakeKeys } = await import("../testing/fake-device-api");
const { createFakeWorkspace } = await import("../testing/fake-workspace");

const { mira: MIRA } = SAMPLE_PEOPLE;
const NEW_DEVICE = "a41f7c2e-6b0d-4e3a-9c85-2d7e1f0b4a96";
const PASSWORD = "a long device password";

interface Download {
	name: string;
	text: string;
}

/** What `saveTextFile` hands to the browser: recorded instead of downloaded. */
function recordDownloads() {
	const view = window as unknown as typeof globalThis;
	const blobs = new Map<string, Blob>();
	const pending: Promise<void>[] = [];
	const downloads: Download[] = [];
	const createObjectURL = URL.createObjectURL;
	const revokeObjectURL = URL.revokeObjectURL;
	const anchorClick = view.HTMLAnchorElement.prototype.click;
	let next = 0;
	URL.createObjectURL = (blob: Blob) => {
		const url = `blob:test-${next++}`;
		blobs.set(url, blob);
		return url;
	};
	URL.revokeObjectURL = () => undefined;
	view.HTMLAnchorElement.prototype.click = function click(
		this: HTMLAnchorElement,
	) {
		const blob = blobs.get(this.getAttribute("href") ?? "");
		const name = this.download;
		if (blob && name)
			pending.push(
				blob.text().then((text) => {
					downloads.push({ name, text });
				}),
			);
	};
	return {
		async all() {
			await Promise.all(pending);
			return downloads;
		},
		restore() {
			URL.createObjectURL = createObjectURL;
			URL.revokeObjectURL = revokeObjectURL;
			view.HTMLAnchorElement.prototype.click = anchorClick;
		},
	};
}

let recorder: ReturnType<typeof recordDownloads> | undefined;

afterEach(async () => {
	recorder?.restore();
	recorder = undefined;
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
	globalThis.localStorage?.clear();
});
afterAll(dom.restore);

function Routed() {
	const { route, scope } = useDevicesRoute();
	return <AccessScreen route={route} scope={scope} />;
}

const mount = (options: Parameters<typeof mountAccess>[1] = {}) =>
	mountAccess(<Routed />, { search: "view=access&tab=shared", ...options });

const sharedRow = (container: HTMLElement) =>
	container.querySelector(
		`[data-shared-device="${SAMPLE_IDS.lab}"]`,
	) as HTMLElement;

const vaultIds = (fake: FakeWorkspace) =>
	fake.workspace.local
		.summary()
		.vaults.map((vault) => vault.deviceId)
		.sort();

/** The connection file an owner downloads for one of their devices. */
function connectionFile(
	fake: FakeWorkspace,
	deviceId: string,
	details: { ownerId: string; name: string },
): File {
	const manifest = fake.hub.manifest(deviceId, details);
	const receipt = {
		enrollment_id: manifest.enrollment_id,
		device_id: deviceId,
		owner_id: details.ownerId,
		name: details.name,
		identity: fakeKeys.identity(deviceId),
		manifest_jws: fakeCompact(manifest),
		binding_jws: fakeCompact({ device_id: deviceId }),
		registered_at: manifest.issued_at + 60,
		auth_epoch: 1,
	};
	return new File(
		[
			JSON.stringify({
				version: 1,
				receipt,
				owner_controller_key: manifest.controller_key,
			}),
		],
		`flow-like-connection-${deviceId}.json`,
		{ type: "application/json" },
	);
}

async function openRequest(mounted: MountedDevices) {
	await click(byRole("button", "Request shared access", mounted.container));
	return inPortal("dialog");
}

async function importConnection(mounted: MountedDevices, file: File) {
	const drop = inPortal("dialog").querySelector(
		"label[for=access-request-file]",
	) as HTMLElement;
	await dropFiles(drop, [file]);
	await mounted.settle();
	return inPortal("dialog");
}

describe("Access › Shared with me", () => {
	test("what the owner gave me, requests still waiting, and where I only pay", async () => {
		const { container } = await mount();
		const row = sharedRow(container);
		expect(row.textContent).toContain("lab-gpu-02");
		expect(row.textContent).toContain("Mira Novak");
		expect(row.querySelector("[data-permissions]")?.textContent).toBe(
			"Custom · 6 permissions",
		);
		// Locked: the sandbox of the device is not known yet, and the row says so.
		expect(
			row.querySelector("[data-code-line]")?.getAttribute("data-code-line"),
		).toBe("unknown");
		expect(row.querySelector("[data-scope=app]")?.textContent).toBe(
			"App Invoice AI",
		);
		expect(row.querySelector("[data-scope=app] a")?.getAttribute("href")).toBe(
			`/library/config/devices?id=${SAMPLE_APPS.invoiceAi}`,
		);
		expect(row.querySelector("[data-ends-soon]")).toBeTruthy();
		expect(row.textContent).toContain("Active");
		expect(row.textContent).toContain("Shared-access keys · locked");
		// The owner key this computer pinned reads without unlocking, in blocks of four with its case kept.
		expect(
			row.querySelector("[data-key-fingerprint] button")?.textContent,
		).toMatch(/^(\S{4} ){4}…$/);
		expect(
			row.querySelector("[data-key-fingerprint] button")?.textContent,
		).toMatch(/[a-z]/);
		expect(
			container.querySelector("#access-shared [data-stamp]")?.textContent,
		).toContain("device list checked");

		const pending = container.querySelector(
			`[data-pending-request="${SAMPLE_IDS.miraRender}"]`,
		) as HTMLElement;
		expect(pending.textContent).toContain("mira-render-01");
		expect(pending.textContent).toContain("Waiting for approval");
		expect(pending.textContent).toContain(
			"Only on this computer until Mira approves.",
		);
		expect(pending.textContent).toContain("Mira chooses them when approving");

		expect(container.querySelector("[data-cloud-only]")?.textContent).toContain(
			"partner-edge",
		);
		expect(container.textContent).toContain(
			"Ask for access to someone's device",
		);
		expect(container.querySelectorAll("[data-dv-primary]").length).toBe(1);
		expect(container.textContent).not.toMatch(MACHINE_WORDS);
		await clickByText("See Cloud approvals & spending", container);
		expect(
			byRole("tab", /Cloud approvals & spending/).getAttribute("aria-selected"),
		).toBe("true");
	});

	test("an older hub: no own-access route, so permissions show after unlock and nothing errors", async () => {
		const mounted = await mount({ hubVersion: "old" });
		const { container, fake } = mounted;
		const row = sharedRow(container);
		expect(row.querySelector("[data-own-access=unknown]")?.textContent).toBe(
			"Shows once unlocked",
		);
		expect(container.querySelector("[role=alert]")).toBeNull();
		expect(container.querySelector("[data-kind=error]")).toBeNull();
		expect(
			fake.api.sent("GET", /management\/my-access/).length,
		).toBeLessThanOrEqual(1);
		const { act } = await import("react");
		await act(async () => {
			await fake.unlock(SAMPLE_IDS.lab);
		});
		await mounted.settle();
		expect(
			sharedRow(container).querySelector("[data-permissions]")?.textContent,
		).toBe("Custom · 6 permissions");
		expect(
			fake.api.sent("GET", /management\/my-access/).length,
		).toBeLessThanOrEqual(1);
	});

	test("ask to renew hands out the same request again from the keys here, without new keys", async () => {
		recorder = recordDownloads();
		const mounted = await mount();
		const { container, fake } = mounted;
		const before = vaultIds(fake);
		await click(byRole("button", "Ask to renew", sharedRow(container)));
		await mounted.settle();
		const downloads = await recorder.all();
		expect(downloads).toHaveLength(1);
		expect(downloads[0]?.name).toBe(`device-access-${SAMPLE_IDS.lab}.json`);
		const [request] = JSON.parse(downloads[0]?.text ?? "[]");
		expect(request.user_id).toBe(fake.hub.me);
		expect(request.grant_id).toBe("1229c956-36d6-4fec-8e80-01409a530896");
		expect(request.controller_key).toEqual(
			fakeKeys.controller(fake.hub.me, SAMPLE_IDS.lab),
		);
		expect(vaultIds(fake)).toEqual(before);
		expect(sharedRow(container).textContent).toContain(
			"Send it to Mira Novak; importing it again renews your access.",
		);
		expect(fake.api.sent("PUT", /management\/policy/)).toHaveLength(0);
	});

	test("access that ended stays listed while its keys are here: ask again or remove them", async () => {
		recorder = recordDownloads();
		const mounted = await mount();
		const { container, fake } = mounted;
		expect(container.querySelector("[data-ended-access]")).toBeNull();
		// The owner removed the access: the hub no longer lists the device for this account.
		fake.hub.rows.delete(SAMPLE_IDS.lab);
		const { act } = await import("react");
		await act(async () => {
			await fake.queryClient.invalidateQueries();
		});
		await mounted.settle();
		expect(sharedRow(container)).toBeNull();
		const ended = container.querySelector(
			`[data-ended-access="${SAMPLE_IDS.lab}"]`,
		) as HTMLElement;
		expect(ended.textContent).toContain("lab-gpu-02");
		expect(ended.textContent).toContain("Access ended");
		expect(ended.textContent).toContain(
			"It left your device list: the access ran out or Mira removed it.",
		);
		await click(byRole("button", "Ask to renew", ended));
		await mounted.settle();
		expect((await recorder.all())[0]?.name).toBe(
			`device-access-${SAMPLE_IDS.lab}.json`,
		);
		await click(byRole("button", "Remove from this computer…", ended));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain("Can you undo it?");
		await click(byRole("button", "Remove from this computer", sheet));
		await mounted.settle();
		expect(vaultIds(fake)).not.toContain(SAMPLE_IDS.lab);
		expect(container.querySelector("[data-ended-access]")).toBeNull();
	});

	test("remove from this computer: consequence preview with an undo row, then the keys are gone", async () => {
		const mounted = await mount();
		const { container, fake } = mounted;
		await click(byRole("button", "More for lab-gpu-02", sharedRow(container)));
		await clickByText("Remove from this computer…", inPortal("menu"));
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain(
			"Remove lab-gpu-02 from this computer?",
		);
		expect(sheet.textContent).toContain("Mira Novak isn't told.");
		expect(sheet.textContent).toContain("Can you undo it?");
		expect(vaultIds(fake)).toContain(SAMPLE_IDS.lab);
		await click(byRole("button", "Remove from this computer", sheet));
		await mounted.settle();
		expect(vaultIds(fake)).not.toContain(SAMPLE_IDS.lab);
		expect(sharedRow(container).textContent).toContain("No keys here");

		// Nothing is left to remove: the reason shows next to the control, and nothing else happens.
		await click(byRole("button", "More for lab-gpu-02", sharedRow(container)));
		await clickByText("Remove from this computer…", inPortal("menu"));
		expect(queryByRole("alertdialog")).toBeNull();
		expect(
			sharedRow(container).querySelector("[role=alert], [role=status]")
				?.textContent,
		).toMatch(/keys/i);
	});

	test("without an account backup the keys here are the only copy, so the device name is typed first", async () => {
		const fake = await createFakeWorkspace();
		fake.hub.backups.delete(SAMPLE_IDS.lab);
		const mounted = await mount({ fake });
		const { container } = mounted;
		await click(byRole("button", "More for lab-gpu-02", sharedRow(container)));
		await clickByText("Remove from this computer…", inPortal("menu"));
		let sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain(
			"There is no account backup of these keys. Request access again to get new ones.",
		);
		const remove = byRole("button", "Remove from this computer", sheet);
		expect(remove.getAttribute("aria-disabled")).toBe("true");
		await click(remove);
		await mounted.settle();
		expect(vaultIds(fake)).toContain(SAMPLE_IDS.lab);

		await typeInto(sheet.querySelector("input") as HTMLInputElement, "lab-gpu-02");
		sheet = inPortal("alertdialog");
		await click(byRole("button", "Remove from this computer", sheet));
		await mounted.settle();
		expect(vaultIds(fake)).not.toContain(SAMPLE_IDS.lab);
	});
});

describe("Access › Request access", () => {
	test("connection file → owner key fingerprint → password → request; a pending row appears", async () => {
		recorder = recordDownloads();
		const mounted = await mount();
		const { container, fake } = mounted;
		const before = vaultIds(fake);
		let sheet = await openRequest(mounted);
		expect(sheet.textContent).toContain("Step 1 of 4 · Connection file");
		expect(
			byRole("button", "Continue", sheet).getAttribute("aria-disabled"),
		).toBe("true");
		sheet = await importConnection(
			mounted,
			connectionFile(fake, NEW_DEVICE, {
				ownerId: MIRA,
				name: "mira-render-02",
			}),
		);
		expect(sheet.textContent).toContain(
			"Read on this computer. Nothing was uploaded.",
		);
		await click(byRole("button", "Continue", sheet));

		// The owner key fingerprint comes before any password.
		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 2 of 4 · Check");
		expect(sheet.textContent).toContain("mira-render-02");
		expect(sheet.textContent).toContain("Mira Novak's owner key fingerprint");
		const ownerKey = fakeKeys.controller(MIRA, NEW_DEVICE).x;
		expect(sheet.querySelector("[data-fingerprint]")?.textContent).toContain(
			ownerKey.match(/.{1,4}/g)?.join(" ") ?? "",
		);
		expect(sheet.querySelector("input[type=password]")).toBeNull();
		expect(sheet.textContent).toContain("Know what Mira can give you");
		expect(
			byRole("button", "Continue", sheet).getAttribute("aria-disabled"),
		).toBe("true");
		await click(
			byRole("checkbox", "Mira read out the same fingerprint.", sheet),
		);
		await click(byRole("button", "Continue", inPortal("dialog")));

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Step 3 of 4 · Device password");
		const create = () =>
			byRole("button", "Create keys and request", inPortal("dialog"));
		expect(create().getAttribute("aria-disabled")).toBe("true");
		const [password, repeat] = Array.from(
			sheet.querySelectorAll<HTMLInputElement>("input[type=password]"),
		);
		await typeInto(password as HTMLInputElement, PASSWORD);
		await typeInto(repeat as HTMLInputElement, "something else entirely");
		expect(inPortal("dialog").textContent).toContain(
			"The two passwords don't match yet.",
		);
		expect(create().getAttribute("aria-disabled")).toBe("true");
		await typeInto(repeat as HTMLInputElement, PASSWORD);
		expect(create().getAttribute("aria-disabled")).toBeNull();
		await click(create());
		await mounted.settle();

		sheet = inPortal("dialog");
		expect(sheet.textContent).toContain(
			"Keys created. The request is ready to send.",
		);
		expect(vaultIds(fake)).toEqual([...before, NEW_DEVICE].sort());
		expect(JSON.stringify(fake.api.calls)).not.toContain(PASSWORD);
		await click(byRole("button", "Download access request", sheet));
		const downloads = await recorder.all();
		expect(downloads[0]?.name).toBe(`device-access-${NEW_DEVICE}.json`);
		const [request] = JSON.parse(downloads[0]?.text ?? "[]");
		expect(Object.keys(request).sort()).toEqual([
			"controller_key",
			"grant_id",
			"user_id",
		]);
		expect(request.user_id).toBe(fake.hub.me);
		await click(byRole("button", "Done", inPortal("dialog")));
		expect(queryByRole("dialog")).toBeNull();

		const pending = container.querySelector(
			`[data-pending-request="${NEW_DEVICE}"]`,
		) as HTMLElement;
		expect(pending.textContent).toContain("mira-render-02");
		expect(pending.textContent).toContain("Waiting for approval");
		expect(pending.textContent).toContain("Mira Novak");
	});

	test("a request made earlier is offered again instead of new keys; other keys for the device block the request", async () => {
		recorder = recordDownloads();
		const mounted = await mount();
		const { fake } = mounted;
		const before = vaultIds(fake);
		await openRequest(mounted);
		let sheet = await importConnection(
			mounted,
			connectionFile(fake, SAMPLE_IDS.miraRender, {
				ownerId: MIRA,
				name: "mira-render-01",
			}),
		);
		expect(
			sheet
				.querySelector("[data-file-problem]")
				?.getAttribute("data-file-problem"),
		).toBe("already_requested");
		expect(sheet.textContent).toContain(
			"This computer already has a request for mira-render-01.",
		);
		expect(
			byRole("button", "Continue", sheet).getAttribute("aria-disabled"),
		).toBe("true");
		await click(byRole("button", "Download request again", sheet));
		const downloads = await recorder.all();
		expect(downloads[0]?.name).toBe(
			`device-access-${SAMPLE_IDS.miraRender}.json`,
		);
		expect(JSON.parse(downloads[0]?.text ?? "[]")[0].controller_key).toEqual(
			fakeKeys.controller(fake.hub.me, SAMPLE_IDS.miraRender),
		);
		expect(vaultIds(fake)).toEqual(before);

		// The viewer's own device: the keys here are owner keys.
		sheet = await importConnection(
			mounted,
			connectionFile(fake, SAMPLE_IDS.edge, {
				ownerId: fake.hub.me,
				name: "edge-berlin-01",
			}),
		);
		expect(
			sheet
				.querySelector("[data-file-problem]")
				?.getAttribute("data-file-problem"),
		).toBe("other_keys");
		expect(sheet.textContent).toContain(
			"This computer already holds other keys for edge-berlin-01.",
		);
		expect(queryByRole("button", "Download request again", sheet)).toBeNull();
	});

	test("files that can't be a connection file are refused with the reason", async () => {
		const mounted = await mount();
		const { fake } = mounted;
		await openRequest(mounted);
		let sheet = await importConnection(
			mounted,
			new File(["[]"], "device-access-1.json"),
		);
		expect(sheet.textContent).toContain("This isn't a connection file.");
		const tampered = JSON.parse(
			await connectionFile(fake, NEW_DEVICE, {
				ownerId: MIRA,
				name: "mira-render-02",
			}).text(),
		);
		tampered.owner_controller_key = fakeKeys.controller(
			"usr_other",
			NEW_DEVICE,
		);
		sheet = await importConnection(
			mounted,
			new File([JSON.stringify(tampered)], "flow-like-connection.json"),
		);
		expect(
			sheet
				.querySelector("[data-file-problem]")
				?.getAttribute("data-file-problem"),
		).toBe("bad_signature");
		expect(sheet.textContent).toContain("Nothing was saved.");
		expect(fake.api.writes().filter(([, path]) => /vault/.test(path))).toEqual(
			[],
		);
	});
});
