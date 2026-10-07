import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../../lib/api-error";
import { APPS } from "../../../../../lib/device-management/model/__fixtures__/apps";
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
const { act } = await import("react");
const { deployPrepareSeams } = await import("../use-deploy-prepare");
const kit = await import("../deploy-test-kit");
const { EDGE, STUDIO, VISITOR, CRM, INVOICE, text } = kit;
await preloadDevices();

const nativeExport = deployPrepareSeams.exportCommands;
/** The first check when no chosen event follows Latest. */
const NO_FLOWS =
	"skip: Create flow versions for current edits · none of the chosen events follows Latest";

afterEach(async () => {
	deployPrepareSeams.exportCommands = nativeExport;
	deployPrepareSeams.publish = {};
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
			NO_FLOWS,
			"pass: Reading what's published on the hub",
			"pass: Checking which events can run on devices",
			"pass: Collecting models and packages",
			expect.stringMatching(/^pass: Definitions approved: [0-9a-f]{8}/),
		]);
		expect(page).toContain("Stays in the cloud.");
		// The definitions hold every event a device can run, the unticked schedule included.
		expect(page).toMatch(/3 events · [\d.]+ KiB · hash/);
		expect(view.fake.api.sent("POST", /version\/current/)).toEqual([]);
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
		expect(checks(view.container)[1]).toBe(`fail: ${reason}`);
		expect(text(view.container)).not.toContain("BAD_REQUEST");
		expect(checks(view.container)[2]).toContain("skip:");
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
		expect(checks(view.container)[1]).toBe(
			"pass: Reading what's published on the hub",
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(metadataRequests(view)).toBe(2);
	});

	test("prepare-blocked: a native collection error shows its reason and can be retried", async () => {
		const fake = await createFakeWorkspace();
		fake.api.on("GET", METADATA, () => {
			const metadata = fake.hub.deviceMetadata(VISITOR);
			metadata.documents.app = {
				...(metadata.documents.app as object),
				bits: ["badge-model"],
			};
			return metadata;
		});
		const reason = "The badge model could not be read. Download it again.";
		let failure: { error: string } | null = { error: reason };
		let attempts = 0;
		const source = new TextEncoder().encode(
			JSON.stringify({ version: 1, project_id: VISITOR }),
		);
		deployPrepareSeams.exportCommands = async () => ({
			prepare: async (project) => {
				attempts += 1;
				if (failure) throw failure;
				return {
					export_id: "a0000000-0000-4000-8000-000000000000",
					project_id: project,
					source: "online",
					files: [
						{
							path: `apps/${project}/online-source.json`,
							size: source.length,
						},
					],
					assets: { bit_pins: [], package_pins: [] },
				};
			},
			read: async (_, _path, offset, length) =>
				source.slice(offset, offset + length).buffer,
			release: async () => {},
		});
		const view = await mount({ fake, platform: "desktop" });
		expect(checks(view.container)).toEqual([
			NO_FLOWS,
			"pass: Reading what's published on the hub",
			"pass: Checking which events can run on devices",
			`fail: ${reason}`,
			"skip: Definitions approved · not checked",
		]);
		expect(kit.footBlocking(view.container)).toBe(reason);
		expect(text(view.container)).not.toMatch(/\[object Object\]|\{"error":/);
		expect(
			byRole("button", "Continue", view.container).getAttribute(
				"aria-disabled",
			),
		).toBe("true");
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		expect(attempts).toBe(1);

		failure = null;
		await clickByText("Prepare again", view.container);
		await view.settle();
		expect(checks(view.container)[3]).toBe(
			"pass: Collecting models and packages",
		);
		expect(checks(view.container)[4]).toMatch(
			/^pass: Definitions approved: [0-9a-f]{8}/,
		);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(
			byRole("button", "Continue", view.container).getAttribute(
				"aria-disabled",
			),
		).toBeNull();
		expect(attempts).toBe(2);
	});

	test("Review shows the native preparation reason and sends the user back to How it runs", async () => {
		const fake = await createFakeWorkspace();
		fake.api.on("GET", METADATA, () => {
			const metadata = fake.hub.deviceMetadata(VISITOR);
			metadata.documents.app = {
				...(metadata.documents.app as object),
				bits: ["badge-model"],
			};
			return metadata;
		});
		const reason = "The badge model could not be read. Download it again.";
		deployPrepareSeams.exportCommands = async () => ({
			prepare: async () => {
				throw { error: reason };
			},
			read: async () => new ArrayBuffer(0),
			release: async () => {},
		});
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: EDGE, step: "review" },
			{ fake, platform: "desktop" },
		);
		const page = text(view.container);
		expect(page).toContain("How it runs needs your attention first.");
		expect(page).toContain(reason);
		for (const root of [
			view.container.querySelector('ol[aria-label="Deploy steps"]'),
			byRole("region", "Your choices", view.container),
		]) {
			expect(
				root?.querySelectorAll("li[data-s]")[1]?.getAttribute("data-s"),
			).toBe("err");
		}
		expect(page).not.toMatch(/\[object Object\]|\{"error":/);
		expect(kit.primaries(view.container)).toBe(0);
		expect(fake.api.sent("POST", /resource-grants/)).toEqual([]);
		expect(fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
			[],
		);
		const writes = fake.api.writes().length;
		await clickByText("Go to How it runs", view.container);
		await view.settle();
		expect(view.navigations.at(-1)?.href).toContain("step=how");
		expect(checks(view.container)[3]).toBe(`fail: ${reason}`);
		expect(kit.footBlocking(view.container)).toBe(reason);
		expect(fake.api.writes().length).toBe(writes);
	});

	test("learning model-store support keeps the approved copy until the chosen events change", async () => {
		const fake = await createFakeWorkspace();
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [EDGE], mode: "new" },
			reached: 6,
		});
		fake.api.on("GET", METADATA, () => {
			const metadata = fake.hub.deviceMetadata(VISITOR);
			metadata.documents.app = {
				...(metadata.documents.app as object),
				bits: ["badge-model"],
			};
			return metadata;
		});
		const formats: boolean[] = [];
		let releases = 0;
		const source = new TextEncoder().encode(
			JSON.stringify({ version: 1, project_id: VISITOR }),
		);
		deployPrepareSeams.exportCommands = async (_app, _account, modelStore) => ({
			prepare: async (project) => {
				formats.push(modelStore === true);
				return {
					export_id: "a0000000-0000-4000-8000-000000000000",
					project_id: project,
					source: "online",
					files: [
						{
							path: `apps/${project}/online-source.json`,
							size: source.length,
						},
					],
					assets: { bit_pins: [], package_pins: [] },
				};
			},
			read: async (_, _path, offset, length) =>
				source.slice(offset, offset + length).buffer,
			release: async () => {
				releases += 1;
			},
		});
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: EDGE, step: "how" },
			{ fake, platform: "desktop" },
		);
		const hash = () =>
			view.container
				.querySelector("li[data-state=pass] [data-idref] button[title]")
				?.getAttribute("title");
		const approved = hash();
		expect(approved).toMatch(/^[0-9a-f]{64}$/);
		expect(formats).toEqual([false]);
		await click(byRole("button", "Go to step 7: Review", view.container));
		await view.settle();

		await act(async () => {
			const agent = fake.api.agent(EDGE);
			agent.features = { ...agent.features, model_store: 1 };
			await fake.workspace.live.refreshInspection(EDGE);
		});
		await view.settle();
		expect(
			fake.workspace.live.inspection(EDGE)?.value.features?.model_store,
		).toBe(1);
		for (const root of [
			view.container.querySelector('ol[aria-label="Deploy steps"]'),
			byRole("region", "Your choices", view.container),
		]) {
			expect(
				root?.querySelectorAll("li[data-s]")[1]?.getAttribute("data-s"),
			).toBe("done");
		}
		expect(formats).toEqual([false]);
		expect(metadataRequests(view)).toBe(1);
		expect(releases).toBe(0);

		await click(byRole("button", "Go to step 2: How it runs", view.container));
		await view.settle();
		expect(hash()).toBe(approved);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(formats).toEqual([false]);
		expect(metadataRequests(view)).toBe(1);
		expect(releases).toBe(0);

		await click(byRole("button", "Go to step 1: What to run", view.container));
		await view.settle();
		await click(
			view.container.querySelector(
				"#deploy-event-evt_visitor_report",
			) as Element,
		);
		await clickByText("Continue", view.container);
		await view.settle();
		expect(checks(view.container)[4]).toMatch(
			/^pass: Definitions approved: [0-9a-f]{8}/,
		);
		expect(formats).toEqual([false, true]);
		expect(metadataRequests(view)).toBe(2);
		expect(releases).toBe(1);
	});

	test("losing model-store support prepares a compatible copy; returning from Review keeps its failure", async () => {
		const fake = await createFakeWorkspace();
		const agent = fake.api.agent(EDGE);
		agent.features = { ...agent.features, model_store: 1 };
		await fake.workspace.live.refreshInspection(EDGE);
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [EDGE], mode: "new" },
			reached: 6,
		});
		fake.api.on("GET", METADATA, () => {
			const metadata = fake.hub.deviceMetadata(VISITOR);
			metadata.documents.app = {
				...(metadata.documents.app as object),
				bits: ["badge-model"],
			};
			return metadata;
		});
		const reason = "The badge model has no known size. Refresh its metadata.";
		const formats: boolean[] = [];
		let releases = 0;
		let finishPreparation = () => {};
		const preparation = new Promise<void>((resolve) => {
			finishPreparation = resolve;
		});
		const source = new TextEncoder().encode(
			JSON.stringify({ version: 1, project_id: VISITOR }),
		);
		deployPrepareSeams.exportCommands = async (_app, _account, modelStore) => ({
			prepare: async (project) => {
				formats.push(modelStore === true);
				if (!modelStore) {
					await preparation;
					throw { error: reason };
				}
				return {
					export_id: "a0000000-0000-4000-8000-000000000000",
					project_id: project,
					source: "online",
					files: [
						{
							path: `apps/${project}/online-source.json`,
							size: source.length,
						},
					],
					assets: { bit_pins: [], package_pins: [] },
				};
			},
			read: async (_, _path, offset, length) =>
				source.slice(offset, offset + length).buffer,
			release: async () => {
				releases += 1;
			},
		});
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: EDGE, step: "how" },
			{ fake, platform: "desktop" },
		);
		expect(checks(view.container)[4]).toMatch(
			/^pass: Definitions approved: [0-9a-f]{8}/,
		);
		expect(formats).toEqual([true]);
		await click(byRole("button", "Go to step 7: Review", view.container));
		await view.settle();
		expect(formats).toEqual([true]);
		const howStates = () =>
			[
				view.container.querySelector('ol[aria-label="Deploy steps"]'),
				byRole("region", "Your choices", view.container),
			].map((root) =>
				root?.querySelectorAll("li[data-s]")[1]?.getAttribute("data-s"),
			);
		expect(howStates()).toEqual(["done", "done"]);

		await act(async () => {
			const { model_store: _modelStore, ...features } = agent.features;
			agent.features = features;
			await fake.workspace.live.refreshInspection(EDGE);
		});
		await view.settle();
		expect(howStates()).toEqual(["busy", "busy"]);
		expect(text(byRole("region", "Your choices", view.container))).toContain(
			"Preparing on this computer…",
		);
		await act(async () => finishPreparation());
		await view.settle();
		expect(formats).toEqual([true, false]);
		expect(releases).toBe(1);
		expect(metadataRequests(view)).toBe(2);
		expect(text(view.container)).toContain(
			"How it runs needs your attention first.",
		);
		expect(text(view.container)).toContain(reason);
		expect(howStates()).toEqual(["err", "err"]);
		expect(text(byRole("region", "Your choices", view.container))).toContain(
			reason,
		);

		await clickByText("Go to How it runs", view.container);
		await view.settle();
		expect(checks(view.container)[3]).toBe(`fail: ${reason}`);
		expect(kit.footBlocking(view.container)).toBe(reason);
		expect(formats).toEqual([true, false]);
		expect(metadataRequests(view)).toBe(2);
		expect(fake.api.commands.filter(([, type]) => type === "apply")).toEqual(
			[],
		);
	});

	test("a device reconnect keeps its model-store capability and prepared copy", async () => {
		let reconnect: () => void = () => {
			throw new Error("No reconnect was scheduled.");
		};
		const fake = await createFakeWorkspace(undefined, {
			workspace: {
				live: {
					schedule: (run, ms) => {
						if (ms === 1_000) {
							reconnect = run;
							return () => {};
						}
						const timer = setTimeout(run, ms);
						return () => clearTimeout(timer);
					},
				},
			},
		});
		const agent = fake.agent(EDGE);
		agent.features = { ...agent.features, model_store: 1 };
		await fake.workspace.live.refreshInspection(EDGE);
		kit.seedDraft(fake, {
			appId: VISITOR,
			scope: { kind: "app", appId: VISITOR },
			route: { deviceIds: [EDGE], mode: "new" },
			reached: 6,
		});
		fake.api.on("GET", METADATA, () => {
			const metadata = fake.hub.deviceMetadata(VISITOR);
			metadata.documents.app = {
				...(metadata.documents.app as object),
				bits: ["badge-model"],
			};
			return metadata;
		});
		const formats: boolean[] = [];
		const source = new TextEncoder().encode(
			JSON.stringify({ version: 1, project_id: VISITOR }),
		);
		deployPrepareSeams.exportCommands = async (_app, _account, modelStore) => ({
			prepare: async (project) => {
				formats.push(modelStore === true);
				return {
					export_id: "a0000000-0000-4000-8000-000000000000",
					project_id: project,
					source: "online",
					files: [
						{
							path: `apps/${project}/online-source.json`,
							size: source.length,
						},
					],
					assets: { bit_pins: [], package_pins: [] },
				};
			},
			read: async (_, _path, offset, length) =>
				source.slice(offset, offset + length).buffer,
			release: async () => {},
		});
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{ device: EDGE, step: "how" },
			{ fake, platform: "desktop" },
		);
		const approved = checks(view.container)[4];
		expect(approved).toMatch(/^pass: Definitions approved: [0-9a-f]{8}/);
		expect(formats).toEqual([true]);
		await click(byRole("button", "Go to step 7: Review", view.container));
		await view.settle();
		const inspection = fake.workspace.live.inspection(EDGE);

		await act(async () => agent.disconnect());
		await view.settle();
		expect(fake.workspace.live.state(EDGE).kind).toBe("reconnecting");
		expect(fake.workspace.live.inspection(EDGE)).toBe(inspection);
		expect(inspection?.value.features?.model_store).toBe(1);
		expect(formats).toEqual([true]);
		await act(async () => reconnect());
		await view.settle();
		expect(fake.workspace.live.state(EDGE).kind).toBe("live");
		expect(fake.workspace.live.inspection(EDGE)).not.toBe(inspection);
		expect(
			fake.workspace.live.inspection(EDGE)?.value.features?.model_store,
		).toBe(1);
		for (const root of [
			view.container.querySelector('ol[aria-label="Deploy steps"]'),
			byRole("region", "Your choices", view.container),
		]) {
			expect(
				root?.querySelectorAll("li[data-s]")[1]?.getAttribute("data-s"),
			).toBe("done");
		}
		await click(byRole("button", "Go to step 2: How it runs", view.container));
		await view.settle();
		expect(checks(view.container)[4]).toBe(approved);
		expect(formats).toEqual([true]);
		expect(metadataRequests(view)).toBe(1);
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
		expect(checks(view.container)[2]).toBe(
			"fail: Badge printer isn't in what the hub publishes for devices. Check it in Events, then prepare again.",
		);
		expect(checks(view.container)[1]).toContain("pass:");
	});

	test("the export is asked for the new types the deploy needs: none for Pages, generic_form for a form", async () => {
		const page = await mount();
		expect(
			page.fake.api.sent("GET", /device-metadata/).map(([, path]) => path),
		).toEqual([METADATA]);
		await cleanupDevices();
		await dom.cleanup();

		const SHOP = "app_shop_assistant";
		const form = await kit.mountApp(mountDevices, SHOP, {
			device: EDGE,
			event: "evt_shop_return",
			step: "how",
		});
		await form.settle();
		expect(
			form.fake.api.sent("GET", /device-metadata/).map(([, path]) => path),
		).toEqual([`apps/${SHOP}/device-metadata?types=generic_form`]);
		expect(checks(form.container)).toContain(
			"pass: Checking which events can run on devices",
		);
	});

	test("a preparation that ends after the wizard is gone changes nothing", async () => {
		const fake = await createFakeWorkspace();
		const release = fake.api.hold({ method: "GET", path: METADATA });
		const view = await mount({ fake });
		expect(checks(view.container)[1]).toContain("active:");
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
			NO_FLOWS,
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

	test.each(["Error", "native error"])(
		"prepare-blocked: a saved secret value fails its own check (%s)",
		async (kind) => {
			const reason =
				"The flow Sync contacts has a saved secret value. Move it into a secret variable, then prepare again.";
			crmExport({
				prepare: async () => {
					throw kind === "Error" ? new Error(reason) : { error: reason };
				},
			});
			const view = await mount();
			expect(checks(view.container)).toEqual([
				NO_FLOWS,
				"pass: Reading the app on this computer",
				`fail: ${reason}`,
				"skip: No flow needs table history or search indexes · not checked",
				"skip: Nothing else is writing to the app · not checked",
				"skip: Under the limits: 8 GiB and 8,192 files · not checked",
			]);
			expect(kit.footBlocking(view.container)).toBe(reason);
			expect(byRole("button", "Prepare again", view.container)).toBeTruthy();
		},
	);

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
		expect(checks(view.container)[1]).toBe(
			"fail: Select the project folder rather than individual files.",
		);
		await clickByText("Use the app on this computer again", view.container);
		await view.settle();
		expect(checks(view.container)[1]).toBe(
			"pass: Reading the app on this computer",
		);
	});
});

describe("How it runs · events that follow Latest", () => {
	const REVIEW = "evt_invoice_review";
	const FLOW = `${INVOICE}/flow_review`;
	const EXPORT = `apps/${INVOICE}/device-metadata?latest=${REVIEW}`;
	const published = (fake: {
		api: { sent(method: "POST", path: RegExp): unknown[] };
	}) => fake.api.sent("POST", /version\/current/).length;
	const mount = (options = {}) =>
		kit.mountApp(
			mountDevices,
			INVOICE,
			{ device: [STUDIO], step: "how" },
			options,
		);

	test("a version equals the flow: nothing is created, and the export is asked for that event", async () => {
		const view = await mount();
		expect(checks(view.container)[0]).toBe(
			"pass: Nothing changed in Review flow since 0.9.2",
		);
		expect(
			checks(view.container)
				.slice(1)
				.every((row) => row.startsWith("pass:")),
		).toBe(true);
		expect(
			view.fake.api.sent("GET", /device-metadata/).map(([, path]) => path),
		).toEqual([EXPORT]);
		expect(published(view.fake)).toBe(1);
		expect(view.fake.hub.flows.state(INVOICE, "flow_review").newest).toEqual([
			0, 9, 2,
		]);
	});

	test("flow edits no version holds: preparing creates the version and says which", async () => {
		const fake = await createFakeWorkspace();
		fake.api.hub.flows.edit(INVOICE, "flow_review");
		const view = await mount({ fake });
		expect(checks(view.container)[0]).toBe(
			"pass: Created flow version 0.9.3 of Review flow",
		);
		expect(fake.hub.flows.state(INVOICE, "flow_review")).toEqual({
			current: [0, 9, 3],
			newest: [0, 9, 3],
		});
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("someone is editing the flow: three more tries, then it stops and says so", async () => {
		deployPrepareSeams.publish = { delayMs: 0 };
		const fake = await createFakeWorkspace();
		fake.api.hub.flows.edit(INVOICE, "flow_review");
		fake.api.hub.flows.locked.add(FLOW);
		const view = await mount({ fake });
		expect(checks(view.container)[0]).toBe(
			"fail: Someone is editing this flow right now. Try again in a moment.",
		);
		expect(published(fake)).toBe(4);
		expect(view.fake.api.sent("GET", /device-metadata/)).toEqual([]);
		fake.api.hub.flows.locked.delete(FLOW);
		await clickByText("Prepare again", view.container);
		await view.settle();
		expect(checks(view.container)[0]).toContain("pass: Created flow version");
	});

	test("a role that can't create a flow version: stops with who can, unless a version already equals the flow", async () => {
		const fake = await createFakeWorkspace();
		fake.api.hub.flows.canPublish = false;
		const unchanged = await mount({ fake });
		expect(checks(unchanged.container)[0]).toBe(
			"pass: Nothing changed in Review flow since 0.9.2",
		);
		await unchanged.unmount();
		globalThis.sessionStorage.clear();
		const edited = await createFakeWorkspace();
		edited.api.hub.flows.canPublish = false;
		edited.api.hub.flows.edit(INVOICE, "flow_review");
		const view = await mount({ fake: edited });
		expect(checks(view.container)[0]).toBe(
			"fail: This flow has edits that aren't published as a version, and your role can't create one. Ask someone who can edit the app to create a version.",
		);
	});

	test("the flow changed between publishing and the export: Prepare again", async () => {
		const fake = await createFakeWorkspace();
		const release = fake.api.hold({
			method: "GET",
			path: `apps/${INVOICE}/device-metadata`,
		});
		const view = await mount({ fake });
		fake.api.hub.flows.edit(INVOICE, "flow_review");
		release();
		await view.settle();
		expect(checks(view.container)[2]).toBe(
			"fail: Review queue's flow changed while preparing. Prepare again.",
		);
		await clickByText("Prepare again", view.container);
		await view.settle();
		expect(checks(view.container)[0]).toBe(
			"pass: Created flow version 0.9.3 of Review flow",
		);
		expect(checks(view.container)[2]).toContain("pass:");
	});

	/** CRM Sync with its webhook following Latest: a local-only app, published and exported on this computer. */
	const crmLatest = () => ({
		...APPS,
		[CRM]: {
			...APPS.app_crm_sync,
			events: APPS.app_crm_sync.events.map((event) =>
				event.id === "evt_crm_webhook"
					? { ...event, board_version: null }
					: event,
			),
		},
	});
	const mountLocal = (latest: unknown) => {
		crmExport({
			prepare: async (project) => ({
				export_id: "a0000000-0000-4000-8000-000000000000",
				project_id: project,
				files: [{ path: `apps/${CRM}/manifest.app`, size: 4 }],
				assets: { bit_pins: [], package_pins: [] },
				latest_events: latest,
			}),
			read: async () => new Uint8Array(4).buffer,
		});
		return kit.mountApp(
			mountDevices,
			CRM,
			{ device: [EDGE], step: "how" },
			{ platform: "desktop", apps: crmLatest() },
		);
	};

	test("a local-only app publishes on this computer and reads the pin from its export", async () => {
		const view = await mountLocal([
			{
				event_id: "evt_crm_webhook",
				board_id: "flow_main",
				board_version: [4, 1, 0],
				problem: null,
			},
		]);
		expect(checks(view.container)[0]).toBe(
			"pass: Nothing changed in Main flow since 4.1.0",
		);
		expect(checks(view.container)[1]).toBe(
			"pass: Reading the app on this computer",
		);
		// Nothing of a local-only app's flows goes through the hub.
		expect(view.fake.api.sent("POST", /version\/current/)).toEqual([]);
	});

	test("a local-only export that left the event out says why: its target is gone, or the flow moved", async () => {
		const gone = await mountLocal([
			{
				event_id: "evt_crm_webhook",
				board_id: "flow_main",
				board_version: null,
				problem: "target_missing",
			},
		]);
		expect(checks(gone.container)[1]).toBe(
			"fail: This event points at a Page or a start node that is no longer in its flow. Open it in Events and pick it again.",
		);
		await gone.unmount();
		globalThis.sessionStorage.clear();
		const moved = await mountLocal([
			{
				event_id: "evt_crm_webhook",
				board_id: "flow_main",
				board_version: null,
				problem: "edited",
			},
		]);
		expect(checks(moved.container)[1]).toBe(
			"fail: CRM webhook's flow changed while preparing. Prepare again.",
		);
	});

	test("its Page or start node left the flow: the reason, and the way to Events, not Prepare again", async () => {
		const fake = await createFakeWorkspace();
		fake.api.hub.unfitEvents.add(REVIEW);
		const view = await mount({ fake });
		expect(checks(view.container)[2]).toBe(
			"fail: This event points at a Page or a start node that is no longer in its flow. Open it in Events and pick it again.",
		);
		expect(
			byRole("link", "Open Events", view.container).getAttribute("href"),
		).toBe(`/library/config/events?id=${INVOICE}&event=${REVIEW}`);
		expect(view.container.textContent).not.toContain("Prepare again");
	});
});
