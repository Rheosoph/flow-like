import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../../lib/api-error";
import type { ExportCommands } from "../../../../../lib/device-management/project-export";
import {
	byRole,
	click,
	clickByText,
	installDom,
	typeInto,
} from "../../testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { deployPrepareSeams } = await import("../use-deploy-prepare");
const kit = await import("../deploy-test-kit");
const { EDGE, STUDIO, VISITOR, CRM, text } = kit;
await preloadDevices();

const nativeExport = deployPrepareSeams.exportCommands;

afterEach(async () => {
	deployPrepareSeams.exportCommands = nativeExport;
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const METADATA = `apps/${VISITOR}/device-metadata`;
const metadataRequests = (view: { fake: { api: { calls: unknown[][] } } }) =>
	view.fake.api.calls.filter(([, path]) => path === METADATA).length;
const checks = (root: ParentNode) =>
	Array.from(root.querySelectorAll("li[data-state]")).map(
		(item) =>
			`${item.getAttribute("data-state")}: ${text(item.children[1] as HTMLElement)}`,
	);

/** A native export of the local-only CRM app: a manifest, one table and a files folder. */
function crmExport(overrides: Partial<ExportCommands> = {}) {
	const files = new Map([
		[`apps/${CRM}/manifest.app`, new Uint8Array(2048).fill(1)],
		[`apps/${CRM}/storage/contacts/data`, new Uint8Array(40_000).fill(2)],
		[`apps/${CRM}/upload/templates/mail.txt`, new Uint8Array(900).fill(3)],
	]);
	const calls = { prepare: 0, release: 0 };
	const commands: ExportCommands = {
		prepare: async (project) => {
			calls.prepare += 1;
			return {
				export_id: "a0000000-0000-4000-8000-000000000000",
				project_id: project,
				files: [...files].map(([path, bytes]) => ({
					path,
					size: bytes.length,
				})),
				assets: { bit_pins: [], package_pins: [] },
			};
		},
		read: async (_, path, offset, length) =>
			(files.get(path) as Uint8Array).slice(offset, offset + length).buffer,
		release: async () => {
			calls.release += 1;
		},
		...overrides,
	};
	deployPrepareSeams.exportCommands = async () => commands;
	return calls;
}

describe("How it runs · online (APP §3.6)", () => {
	const mount = (options = {}) =>
		kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: [EDGE, STUDIO], step: "how" },
			options,
		);

	test("the mode is a fact, and the version is prepared once", async () => {
		const view = await mount();
		const page = text(view.container);
		expect(page).toContain(
			"Decided by the app: Visitor Check-in is an online app.",
		);
		expect(
			view.container
				.querySelector("[data-mode-card]")
				?.getAttribute("data-mode-card"),
		).toBe("online");
		expect(page).toContain("Why not the other way?");
		expect(page).toContain(
			"devices can't take a copy of an online app to run without internet yet",
		);
		expect(checks(view.container)).toEqual([
			"pass: Reading what's published on the hub",
			"pass: Checking which events can run on devices",
			"pass: Collecting models and packages",
			expect.stringMatching(/^pass: Definitions approved: [0-9a-f]{8}/),
		]);
		expect(page).toContain("Stays in the cloud.");
		expect(page).toMatch(/2 events · [\d.]+ KiB · hash/);
		expect(page).toMatch(/2 files · [\d.]+ KiB per device/);
		expect(metadataRequests(view)).toBe(1);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.primaries(view.container)).toBe(1);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
		// The prepared definitions feed Settings.
		expect(text(byRole("region", "Your choices"))).toContain("3 app defaults");
	});

	test("prepare-blocked: the hub's reason is the failing check, and Continue waits", async () => {
		const fake = await createFakeWorkspace();
		const reason =
			"Publish widgets before deploying them: Visitor badge is a draft.";
		// As the hub sends it: the error's own text also carries the code and a reference.
		const restore = fake.api.fail(
			{ method: "GET", path: METADATA },
			new ApiResponseError({
				status: 400,
				code: "BAD_REQUEST",
				errorId: "ref-1",
				message: reason,
			}),
		);
		const view = await mount({ fake });
		expect(checks(view.container)[0]).toBe(`fail: ${reason}`);
		expect(text(view.container)).not.toContain("BAD_REQUEST");
		expect(checks(view.container)[1]).toContain("skip:");
		expect(text(view.container)).toContain("· not checked");
		expect(kit.footBlocking(view.container)).toBe(reason);
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		// One request per attempt: no retry storm.
		expect(metadataRequests(view)).toBe(1);

		restore();
		await clickByText("Prepare again", view.container);
		await view.settle();
		expect(checks(view.container)[0]).toBe(
			"pass: Reading what's published on the hub",
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(metadataRequests(view)).toBe(2);
	});

	test("an event the hub doesn't publish for devices stops the preparation", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const hub = view.fake.hub;
		hub.apps = {
			...hub.apps,
			[VISITOR]: {
				...hub.apps[VISITOR],
				events: hub.apps[VISITOR].events.filter(
					(event) => event.id !== "evt_badge_printer",
				),
			},
		};
		await clickByText("Continue", view.container);
		await view.settle();
		expect(checks(view.container)[1]).toBe(
			"fail: Badge printer isn't in what the hub publishes for devices. Check it in Events, then prepare again.",
		);
		expect(checks(view.container)[0]).toContain("pass:");
	});

	test("a preparation that ends after the wizard is gone changes nothing", async () => {
		const fake = await createFakeWorkspace();
		const release = fake.api.hold({ method: "GET", path: METADATA });
		const view = await mount({ fake });
		expect(checks(view.container)[0]).toContain("active:");
		expect(kit.footBlocking(view.container)).toBe(
			"Preparing on this computer…",
		);
		await view.unmount();
		release();
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(document.body.textContent).not.toContain("Definitions approved");
	});
});

describe("How it runs · offline copy", () => {
	const mount = (params = {}, options = {}) =>
		kit.mountApp(
			mountDevices,
			CRM,
			{ device: [EDGE, STUDIO], step: "how", ...params },
			{ platform: "desktop", ...options },
		);

	test("the copy is read on this computer and listed with its size", async () => {
		const calls = crmExport();
		const view = await mount();
		const page = text(view.container);
		expect(
			view.container
				.querySelector("[data-mode-card]")
				?.getAttribute("data-mode-card"),
		).toBe("offline");
		expect(checks(view.container)).toEqual([
			"pass: Reading the app on this computer",
			"pass: No saved secret values in flows",
			"pass: No flow needs table history or search indexes",
			"pass: Nothing else is writing to the app",
			expect.stringMatching(
				/^pass: Under the limits: [\d.]+ KiB of 8 GiB, 3 of 8,192 files/,
			),
		]);
		const list = view.container.querySelector("[data-ship-list]");
		expect(text(list as HTMLElement)).toContain("storage/");
		expect(text(list as HTMLElement)).toContain("manifest.app");
		expect(text(list as HTMLElement)).toContain("To each device3 files");
		expect(page).toContain("Data on the device");
		expect(page).toContain(
			"Each new service starts with this copy of the data.",
		);
		expect(calls.prepare).toBe(1);
		expect(text(byRole("region", "Your choices"))).toMatch(
			/Copy & upload[\d.]+ KiB to 2 devices/,
		);
		await view.unmount();
		expect(calls.release).toBe(1);
	});

	test("prepare-blocked: a saved secret value fails its own check", async () => {
		const reason =
			"The flow Sync contacts has a saved secret value. Move it into a secret variable, then prepare again.";
		crmExport({
			prepare: async () => {
				throw new Error(reason);
			},
		});
		const view = await mount();
		expect(checks(view.container)).toEqual([
			"pass: Reading the app on this computer",
			`fail: ${reason}`,
			"skip: No flow needs table history or search indexes · not checked",
			"skip: Nothing else is writing to the app · not checked",
			"skip: Under the limits: 8 GiB and 8,192 files · not checked",
		]);
		expect(kit.footBlocking(view.container)).toBe(reason);
		expect(byRole("button", "Prepare again", view.container)).toBeTruthy();
	});

	test("update-offline: a new copy replaces the app, the data on the device stays", async () => {
		crmExport();
		const view = await kit.mountApp(
			mountDevices,
			CRM,
			{ mode: undefined, device: EDGE, service: "nightly-sync", step: "how" },
			{ platform: "desktop" },
		);
		const page = text(view.container);
		expect(page).toContain("Re-upload the copy");
		expect(page).toContain(
			"Sends a new copy of CRM Sync from this computer to edge-berlin-01. It replaces the app that nightly-sync runs.",
		);
		expect(page).toContain("The data on the device stays.");
		// nightly-sync is stopped, as its owner asked.
		expect(page).toContain(
			"Nothing is running there now, so the update interrupts nothing.",
		);
		expect(page).not.toContain("Import a prepared copy instead");
	});

	test("keeping the version uploads and prepares nothing", async () => {
		const calls = crmExport();
		const view = await kit.mountApp(
			mountDevices,
			CRM,
			{ mode: undefined, device: EDGE, service: "nightly-sync" },
			{ platform: "desktop" },
		);
		await click(byRole("radio", /Keep each service's version/, view.container));
		await clickByText("Continue", view.container);
		await view.settle();
		expect(text(view.container)).toContain("Nothing is uploaded");
		expect(view.container.querySelector("li[data-state]")).toBeNull();
		expect(calls.prepare).toBe(0);
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("a prepared copy can be imported instead; a wrong selection says why", async () => {
		crmExport();
		const view = await mount({ device: EDGE });
		await clickByText("Import a prepared copy instead", view.container);
		const folder = view.container.querySelector(
			"#deploy-import-folder",
		) as HTMLInputElement;
		// A file picked on its own, not as part of a folder.
		const loose = new File(["x"], "manifest.app");
		Object.defineProperty(loose, "webkitRelativePath", { value: "" });
		Object.defineProperty(folder, "files", { value: [loose] });
		await typeInto(folder, "");
		await view.settle();
		expect(checks(view.container)[0]).toBe(
			"fail: Select the project folder rather than individual files.",
		);
		await clickByText("Use the app on this computer again", view.container);
		await view.settle();
		expect(checks(view.container)[0]).toBe(
			"pass: Reading the app on this computer",
		);
	});
});
