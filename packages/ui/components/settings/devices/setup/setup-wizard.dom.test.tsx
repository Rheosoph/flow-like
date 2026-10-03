import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { DeviceSetupInput } from "../../../../lib/device-management/setup";
import type { OnboardingManifest } from "../../../../lib/device-management/types";
import {
	DAY_S,
	type TestReleaseOptions,
	publishTestRelease,
} from "../hub/release-test-kit";
import {
	allByRole,
	byRole,
	click,
	clickByText,
	fire,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type { PrepareSetup } from "./use-setup-create";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { SetupWizard } = await import("./setup-wizard");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { keyFileLogOf } = await import("../keys/key-store");
const { deviceKeys } = await import(
	"../../../../lib/device-management/hub/queries"
);
const { accountStorageKey } = await import(
	"../../../../lib/device-management/storage"
);
const { formatMoment } = await import("../../../../lib/date");

let restoreFetch: (() => void) | undefined;
afterEach(async () => {
	await cleanupDevices();
	restoreFetch?.();
	restoreFetch = undefined;
	await dom.cleanup();
	dom.window.sessionStorage.clear();
});
afterAll(dom.restore);

const PASSWORD = "violet-harbour-lantern-42";
const NAME = "factory-line-4";
const KEY = { kty: "OKP", crv: "Ed25519", x: "A".repeat(43) } as const;
const PENDING = "c382dd52-1611-4477-b983-4e1e1417b669";
const LAPSED = "bd3b57bb-a630-4470-b978-4c6e1ecb5c4f";
const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\b[GD]\d{1,2}\b/;
const MIB = 1024 * 1024;

interface PrepareOptions {
	backup?: "saved" | "local_only" | "limit";
}

/** Stands in for the WASM-backed call: registers with the fake hub like the real one and hands back two files. */
function fakePrepare(options: PrepareOptions = {}) {
	const calls: DeviceSetupInput[] = [];
	const prepare: PrepareSetup = async (input) => {
		calls.push(input);
		input.signal?.throwIfAborted();
		const response = await input.api.fetch<{ manifest: OnboardingManifest }>(
			input.profile,
			"devices/enrollments",
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				signal: input.signal,
				body: JSON.stringify({
					name: input.name,
					bootstrap_key: KEY,
					controller_key: KEY,
					owner_invitation_key: KEY,
				}),
			},
		);
		return {
			package: new Blob(["package"]),
			backup: new Blob(["backup"]),
			deviceId: response.manifest.device_id,
			accountBackup: input.backupToAccount
				? (options.backup ?? "saved")
				: undefined,
			storage: "persisted",
		};
	};
	return { prepare, calls };
}

/** Registers like `fakePrepare`, then keeps building until the test lets it finish. */
function slowPrepare() {
	let finish = () => {};
	const built = new Promise<void>((resolve) => {
		finish = resolve;
	});
	const inner = fakePrepare();
	const prepare: PrepareSetup = async (input) => {
		const result = await inner.prepare(input);
		await built;
		return result;
	};
	return { prepare, finish: () => finish() };
}

function Screen({ prepare }: Readonly<{ prepare: PrepareSetup }>) {
	const { route, scope } = useDevicesRoute();
	return <SetupWizard route={route} scope={scope} harness={{ prepare }} />;
}

async function mountSetup(
	options: MountDevicesOptions & { prepare?: PrepareOptions } = {},
) {
	const { prepare: prepareOptions, ...mount } = options;
	const stub = fakePrepare(prepareOptions);
	const mounted = await mountDevices(<Screen prepare={stub.prepare} />, {
		search: "flow=setup",
		...mount,
	});
	return { ...mounted, ...stub };
}

/* Queries. */

const text = () => document.body.textContent ?? "";
const stepOf = () =>
	Number(
		document
			.querySelector("[data-setup-step]")
			?.getAttribute("data-setup-step"),
	);
function primary(): HTMLButtonElement {
	const button = document.querySelector<HTMLButtonElement>(
		"[data-wizard-foot] [data-dv-primary]",
	);
	if (!button) throw new Error("The wizard foot has no primary button");
	return button;
}
const gated = () => primary().getAttribute("aria-disabled") === "true";
const input = (id: string) => {
	const el = document.getElementById(id);
	if (!el) throw new Error(`No field #${id}`);
	return el;
};
const posts = (fake: FakeWorkspace) =>
	fake.api.sent("POST", "devices/enrollments");
const trayItems = (fake: FakeWorkspace) =>
	fake.workspace.activity.list().filter((item) => item.kind === "setup");
/** What the wizard itself may change at the hub: a setup made or cancelled. */
const setupWrites = (fake: FakeWorkspace) =>
	fake.api
		.writes()
		.filter(([, path]) => path.startsWith("devices/enrollments"));
const storedDrafts = (fake: FakeWorkspace) =>
	dom.window.sessionStorage.getItem(
		`flow-like.devices.setup.${accountStorageKey(fake.scope)}`,
	) ?? "";

/** Fingerprints, ids and addresses are data: random base64url can look like a machine word. */
function copyText(): string {
	const copy = document.body.cloneNode(true) as HTMLElement;
	for (const node of copy.querySelectorAll("[data-fingerprint], [data-idref]"))
		node.remove();
	return copy.textContent ?? "";
}

/** Every screen state: one coral at most (R2) and no machine vocabulary (R3). */
function expectClean() {
	expect(
		document.querySelectorAll("[data-dv-primary]").length,
	).toBeLessThanOrEqual(1);
	expect(copyText()).not.toMatch(MACHINE_WORDS);
}

/* Actions. */

const next = () => click(primary());

/** Enter in a field, or any other way the form is submitted without the button. */
async function submit() {
	const form = document.querySelector("form");
	if (!form) throw new Error("The wizard has no form");
	await fire(
		form,
		new dom.window.Event("submit", {
			bubbles: true,
			cancelable: true,
		}) as unknown as Event,
	);
}

async function recheck(fake: FakeWorkspace) {
	await act(async () => {
		void fake.queryClient.refetchQueries({
			queryKey: deviceKeys.readiness(accountStorageKey(fake.scope)),
		});
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function toPasswordStep(
	platform: RegExp = /Linux \(Intel\/AMD 64-bit\)/,
) {
	await next();
	await typeInto(byRole("textbox", /Name this device/), NAME);
	await next();
	await click(byRole("radio", platform));
	await next();
	expect(stepOf()).toBe(3);
}

async function enterPassword() {
	await typeInto(input("dv-setup-password"), PASSWORD);
	await typeInto(input("dv-setup-repeat"), PASSWORD);
	await next();
}

async function toCreateStep(platform?: RegExp) {
	await toPasswordStep(platform);
	await enterPassword();
	expect(stepOf()).toBe(4);
}

async function create(settle: () => Promise<void>, platform?: RegExp) {
	await toCreateStep(platform);
	await next();
	await settle();
	expect(text()).toContain("Package ready");
}

/* Hub conditions. */

const NOT_READY = (fake: FakeWorkspace) => ({
	version: 1 as const,
	ready: false,
	checks: fake.hub.readiness.checks.map((check) =>
		check.id === "signaling"
			? {
					...check,
					ready: false,
					message: "The hub must configure secure signaling endpoints.",
				}
			: check,
	),
});

const b64url = (bytes: Uint8Array) =>
	btoa(String.fromCharCode(...bytes))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/, "");

/** A release whose agents are too large to pack, with a Linux-only image. */
async function largeAgentHub() {
	const fake = await createFakeWorkspace();
	await publishTestRelease(fake, {
		artifacts: [
			{ target: "x86_64-unknown-linux-gnu", size: 512 * MIB },
			{ target: "aarch64-apple-darwin", size: 512 * MIB },
		],
		platforms: ["linux/amd64"],
	});
	return fake;
}

/** A hub whose release list this file signed, with the validity the test chooses. */
async function hubWithRelease(release: TestReleaseOptions) {
	const fake = await createFakeWorkspace();
	const published = await publishTestRelease(fake, release);
	return { fake, published };
}

/** A moment as the wizard writes it: the time today, the day and time otherwise. */
const moment = (fake: FakeWorkspace, atS: number) =>
	formatMoment(atS * 1000, { now: fake.clock.now(), locale: "en" });

interface HeldFetch {
	signal: AbortSignal | undefined;
	release(): void;
}

/** Keeps requests for one URL pending until the test lets each one through. */
function holdFetch(url: string): HeldFetch[] {
	const inner = globalThis.fetch;
	const held: HeldFetch[] = [];
	globalThis.fetch = ((target: RequestInfo | URL, init?: RequestInit) => {
		if (String(target) !== url) return inner(target, init);
		return new Promise<Response>((resolve, reject) => {
			const signal = init?.signal ?? undefined;
			signal?.addEventListener("abort", () =>
				reject(new DOMException("Aborted", "AbortError")),
			);
			held.push({ signal, release: () => resolve(inner(target, init)) });
		});
	}) as typeof fetch;
	restoreFetch = () => {
		globalThis.fetch = inner;
	};
	return held;
}

/* The first walk, phase by phase. */

type Mounted = Awaited<ReturnType<typeof mountSetup>>;

/** Steps 0–3: an empty Continue earns its message before each choice is made. */
async function walkChoices({ navigations }: Mounted) {
	expect(stepOf()).toBe(0);
	expect(text()).toContain("is ready for a new device");
	expect(text()).toContain("All 6 hub checks pass, agent 0.9.4 is verified");
	expect(text()).toContain("can register devices, that the agent release");
	expectClean();
	expect(gated()).toBe(false);

	await next();
	expect(stepOf()).toBe(1);
	expect(navigations.at(-1)).toEqual({
		mode: "replace",
		href: "/settings/devices?flow=setup&step=1",
	});
	await next();
	expect(text()).toContain("Enter a name for the device.");
	expect(stepOf()).toBe(1);
	await typeInto(byRole("textbox", /Name this device/), NAME);
	expect(text()).toContain("14 of 256 bytes");
	expectClean();

	await next();
	expect(stepOf()).toBe(2);
	await next();
	expect(text()).toContain("Pick the platform the device runs on.");
	await click(byRole("radio", /Linux \(Intel\/AMD 64-bit\)/));
	expectClean();

	await next();
	expect(stepOf()).toBe(3);
	await typeInto(input("dv-setup-password"), "short");
	await next();
	expect(text()).toContain("Use at least 12 bytes. This one has 5.");
	await typeInto(input("dv-setup-password"), PASSWORD);
	await typeInto(input("dv-setup-repeat"), `${PASSWORD}x`);
	await next();
	expect(text()).toContain("The two passwords don't match.");
	expect(stepOf()).toBe(3);
	await typeInto(input("dv-setup-repeat"), PASSWORD);
	expectClean();
}

/** Step 4: nothing reaches the hub before Create, and one setup after it. */
async function walkCreate({ fake, calls, settle }: Mounted) {
	await next();
	expect(stepOf()).toBe(4);
	expect(text()).toContain("What will be created");
	expect(text()).toContain("Before this runs");
	expect(text()).toContain("flow-like-factory-line-4.zip");
	expect(primary().textContent).toContain("Create setup package");
	expect(posts(fake)).toHaveLength(0);
	expectClean();

	await next();
	await settle();
	expect(calls).toHaveLength(1);
	expect(calls[0]).toMatchObject({
		name: NAME,
		mode: "binary",
		target: "x86_64-unknown-linux-gnu",
		password: PASSWORD,
		backupToAccount: true,
	});
	expect(posts(fake)).toHaveLength(1);
	expect(text()).toContain("Package ready");
	expect(text()).toContain("The package for factory-line-4 is ready.");
	expectClean();
}

/** Step 5: Continue stays gated until the package was downloaded. */
async function walkSave() {
	await next();
	expect(stepOf()).toBe(5);
	expect(text()).toContain("Saved to your account");
	expect(text()).toContain("Optional · your account has a copy");
	expect(gated()).toBe(true);
	expect(text()).toContain("Download the setup package first.");
	await submit();
	expect(stepOf()).toBe(5);
	expectClean();
	const download = byRole("link", /Download setup package/);
	expect(download.getAttribute("download")).toBe(
		"flow-like-factory-line-4.zip",
	);
	download.addEventListener("click", (event) => event.preventDefault());
	await click(download);
	expect(gated()).toBe(false);
}

/** Steps 6 and 7, up to the wait for the first check-in. */
async function walkStart() {
	await next();
	expect(stepOf()).toBe(6);
	expect(text()).toContain(
		"unzip flow-like-factory-line-4.zip -d flow-like-factory-line-4",
	);
	expect(text()).toContain("sh start.sh");
	expect(text()).toContain("./flow-like-standalone install-service");
	expect(primary().textContent).toContain("Wait for the device");
	expectClean();

	await next();
	expect(stepOf()).toBe(7);
	expect(text()).toContain("Waiting for factory-line-4 to start.");
	expect(text()).toContain("Opens once factory-line-4 checks in.");
	expect(gated()).toBe(true);
	expectClean();
}

describe("a new setup", () => {
	test("walks from the hub check to the first check-in", async () => {
		const mounted = await mountSetup();
		const { fake, navigations, settle } = mounted;
		await walkChoices(mounted);
		await walkCreate(mounted);
		await walkSave();
		await walkStart();

		const enrollment = fake.hub.enrollments.find((row) => row.name === NAME);
		if (!enrollment) throw new Error("The hub has no enrollment for the setup");
		const deviceKey = deviceKeys.device(
			accountStorageKey(fake.scope),
			enrollment.device_id,
		);
		const poll = () =>
			act(async () => {
				await fake.queryClient.refetchQueries({ queryKey: deviceKey });
			});
		await poll();
		expect(text()).not.toContain("Couldn't load");
		fake.hub.redeem(enrollment.enrollment_id);
		await poll();
		expect(text()).toContain("factory-line-4 registered with the hub.");
		expect(gated()).toBe(true);
		fake.hub.checkIn(enrollment.device_id);
		await poll();
		await settle();
		expect(text()).toContain("factory-line-4 is set up and checked in at");
		expect(text()).toContain("Set up another device");
		expect(gated()).toBe(false);
		expectClean();
		// The tray entry of the setup is done and leads to the device from now on.
		expect(trayItems(fake)).toMatchObject([
			{
				state: "done",
				href: { screen: "device", deviceId: enrollment.device_id },
			},
		]);

		await next();
		expect(navigations.at(-1)).toEqual({
			mode: "push",
			href: `/settings/devices?device=${enrollment.device_id}&tab=overview`,
		});
	});

	test("keeps choices but no secret and no package in sessionStorage; a reloaded window can't download the package again", async () => {
		const first = await mountSetup();
		await create(first.settle);
		await next();
		expect(stepOf()).toBe(5);
		const stored = storedDrafts(first.fake);
		expect(stored).toContain(NAME);
		expect(stored).toContain("x86_64-unknown-linux-gnu");
		for (const store of [dom.window.sessionStorage, dom.window.localStorage])
			for (let index = 0; index < store.length; index++) {
				const value = store.getItem(store.key(index) ?? "") ?? "";
				expect(value).not.toContain(PASSWORD);
				expect(value).not.toContain("blob:");
			}
		await first.unmount();

		const reloaded = await mountSetup({ search: "flow=setup&step=5" });
		expect(stepOf()).toBe(5);
		expect(text()).toContain("This window can't download the package again");
		expect(text()).toContain(
			"This package can't be downloaded again. If you lose it, cancel this setup and create a new one.",
		);
		expect(queryByRole("link", /Download setup package/)).toBeNull();
		expect(gated()).toBe(true);
		expect(text()).toContain("Confirm that you have the package");
		await next();
		expect(stepOf()).toBe(5);
		await click(byRole("checkbox", /I have flow-like-factory-line-4\.zip/));
		expect(gated()).toBe(false);
		await next();
		expect(stepOf()).toBe(6);
		expect(reloaded.calls).toHaveLength(0);
		expectClean();
	});

	test("asks for the device password again after a reload instead of creating", async () => {
		const first = await mountSetup();
		await toCreateStep();
		await first.unmount();

		const reloaded = await mountSetup({ search: "flow=setup&step=4" });
		expect(stepOf()).toBe(4);
		expect(text()).toContain("Enter the device password again.");
		expect(gated()).toBe(true);
		await next();
		await submit();
		expect(reloaded.calls).toHaveLength(0);
		expect(posts(reloaded.fake)).toHaveLength(0);
		await clickByText("Go to Password");
		expect(stepOf()).toBe(3);
	});

	test("a step in the URL never skips a choice that is still missing", async () => {
		await mountSetup({ search: "flow=setup&step=6" });
		expect(stepOf()).toBe(1);
		expect(byRole("textbox", /Name this device/)).toBeDefined();
	});

	test("leaving after the package is built names what is lost, and Cancel keeps the wizard", async () => {
		const { fake, navigations, settle } = await mountSetup();
		await create(settle);
		const before = navigations.length;
		await clickByText("Exit setup");
		const sheet = inPortal();
		expect(sheet.textContent).toContain("Leave the setup for factory-line-4?");
		expect(sheet.textContent).toContain(
			"The package for factory-line-4 is discarded from this window and can't be downloaded again.",
		);
		expect(sheet.textContent).toContain("stays in Pending setups until");
		await clickByText("Cancel", sheet);
		expect(navigations).toHaveLength(before);
		expect(stepOf()).toBe(4);

		await clickByText("Exit setup");
		await clickByText("Leave setup", inPortal());
		await settle();
		expect(navigations.at(-1)).toEqual({
			mode: "push",
			href: "/settings/devices",
		});
		expect(fake.api.sent("DELETE")).toHaveLength(0);
	});

	test("Exit before anything is created leaves without asking", async () => {
		const { navigations } = await mountSetup();
		await next();
		await clickByText("Exit setup");
		expect(queryByRole("dialog")).toBeNull();
		expect(navigations.at(-1)?.href).toBe("/settings/devices");
	});

	test("Cancel setup confirms, cancels at the hub and frees the wizard for a new setup", async () => {
		const { fake, settle } = await mountSetup();
		await create(settle);
		await next();
		await clickByText("Cancel setup…");
		const sheet = inPortal();
		expect(sheet.textContent).toContain("Cancel the setup for factory-line-4?");
		expect(sheet.textContent).toContain(
			"The setup package for factory-line-4 stops working.",
		);
		expect(sheet.textContent).toContain("No, this is permanent.");
		expect(fake.api.sent("DELETE")).toHaveLength(0);
		await clickByText("Cancel setup for factory-line-4", sheet);
		await settle();
		expect(fake.api.sent("DELETE", /^devices\/enrollments\//)).toHaveLength(1);
		expect(text()).toContain("The setup for factory-line-4 is cancelled.");
		expect(queryByRole("link", /Download setup package/)).toBeNull();
		expect(primary().textContent).toContain("Start a new setup");
		expectClean();

		await next();
		await settle();
		expect(stepOf()).toBe(0);
		expect(text()).not.toContain("is cancelled");
	});

	test("a failed account backup makes the key backup file a requirement", async () => {
		const { fake, settle } = await mountSetup({
			prepare: { backup: "local_only" },
		});
		await create(settle);
		expect(text()).toContain("Saving account backup · didn't finish");
		await next();
		expect(text()).toContain("Only on this computer");
		expect(text()).toContain("Required");
		expect(text()).toContain("Save the key backup file first.");
		expect(queryByRole("link", /Download setup package/)).toBeNull();
		const blocked = byRole("button", /Download setup package/);
		expect(blocked.getAttribute("aria-disabled")).toBe("true");
		await click(blocked);
		expect(text()).toContain("Download the key backup file first.");

		const key = byRole("link", /Download key backup file/);
		expect(key.getAttribute("download")).toBe(
			"flow-like-factory-line-4-keys.json",
		);
		key.addEventListener("click", (event) => event.preventDefault());
		await click(key);
		expect(byRole("link", /Download setup package/)).toBeDefined();
		expectClean();
		// Keys & recovery reads "saved at setup" from this computer's record of the download.
		const made = fake.hub.enrollments.find((row) => row.name === NAME);
		expect(keyFileLogOf(fake.workspace).read()[made?.device_id ?? ""]).toEqual({
			savedAt: fake.clock.now(),
		});
	});

	test("leaving the wizard while the package is being made drops the late result", async () => {
		const { fake, calls, unmount } = await mountSetup();
		await toCreateStep();
		const release = fake.api.hold({
			method: "POST",
			path: "devices/enrollments",
		});
		await next();
		expect(calls).toHaveLength(1);
		expect(text()).toContain("Creating the package for factory-line-4.");
		expect(primary().getAttribute("aria-busy")).toBe("true");
		const signal = calls[0]?.signal;
		await unmount();
		expect(signal?.aborted).toBe(true);
		release();
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		expect(storedDrafts(fake)).not.toContain("created");
		expect(fake.hub.enrollments.some((row) => row.name === NAME)).toBe(false);
	});

	test("Exit while the package is being made asks first, cancels at the hub and keeps the choices for coming back", async () => {
		const slow = slowPrepare();
		const { fake, navigations, settle, unmount } = await mountDevices(
			<Screen prepare={slow.prepare} />,
			{ search: "flow=setup" },
		);
		await toCreateStep();
		await next();
		await settle();
		expect(text()).toContain("Creating the package for factory-line-4.");
		expect(posts(fake)).toHaveLength(1);
		expect(trayItems(fake)).toHaveLength(1);
		const before = navigations.length;
		await clickByText("Exit setup");
		const sheet = inPortal();
		expect(sheet.textContent).toContain(
			"Stop creating the package for factory-line-4?",
		);
		expect(sheet.textContent).toContain(
			"Your choices stay in this window if you come back.",
		);
		expect(navigations).toHaveLength(before);
		await clickByText("Stop and cancel setup", sheet);
		slow.finish();
		await settle();
		expect(fake.api.sent("DELETE", /^devices\/enrollments\//)).toHaveLength(1);
		expect(navigations.at(-1)).toEqual({
			mode: "push",
			href: "/settings/devices",
		});
		expect(trayItems(fake)).toHaveLength(0);
		const stored = storedDrafts(fake);
		expect(stored).toContain(NAME);
		expect(stored).not.toContain("cancelledAt");
		expect(stored).not.toContain(PASSWORD);
		await unmount();

		const back = await mountSetup();
		expect(stepOf()).toBe(4);
		expect(text()).toContain("Enter the device password again.");
		expect(text()).toContain("flow-like-factory-line-4.zip");
		expect(gated()).toBe(true);
		expect(back.calls).toHaveLength(0);
	});

	test("a stopped creation the hub can't cancel no longer reads as creating: the hub still holds the setup", async () => {
		const slow = slowPrepare();
		const { fake, navigations, settle } = await mountDevices(
			<Screen prepare={slow.prepare} />,
			{ search: "flow=setup" },
		);
		await toCreateStep();
		await next();
		await settle();
		expect(text()).toContain("Creating the package for factory-line-4.");
		fake.api.fail({ method: "DELETE", path: /^devices\/enrollments\// });
		const before = navigations.length;
		await clickByText("Exit setup");
		await clickByText("Stop and cancel setup", inPortal());
		slow.finish();
		await settle();
		expect(
			fake.api.sent("DELETE", /^devices\/enrollments\//).length,
		).toBeGreaterThan(0);
		// The wizard stays: nothing runs, and the reservation is still at the hub.
		expect(navigations).toHaveLength(before);
		expect(stepOf()).toBe(4);
		expect(text()).not.toContain("Creating the package for factory-line-4.");
		expect(text()).toContain(
			"The hub still holds the half-made setup. Cancel it before you try again.",
		);
		expectClean();
	});

	test("after a failed creation, a changed choice is what gets created, never the request that failed", async () => {
		const fake = await createFakeWorkspace();
		const { calls, settle } = await mountSetup({ fake });
		await toCreateStep();
		const room = fake.hub.limits.max_pending_enrollments;
		fake.hub.limits.max_pending_enrollments = 1;
		await next();
		await settle();
		expect(primary().textContent).toContain("Try again");
		expect(calls).toHaveLength(1);
		fake.hub.limits.max_pending_enrollments = room;

		const back = () => click(byRole("button", /^Back$/));
		await back();
		expect(stepOf()).toBe(3);
		await back();
		await back();
		expect(stepOf()).toBe(1);
		await typeInto(byRole("textbox", /Name this device/), "factory-line-9");
		await next();
		await next();
		await next();
		expect(stepOf()).toBe(4);
		expect(text()).toContain("What will be created");
		expect(text()).toContain("flow-like-factory-line-9.zip");
		expect(text()).not.toContain("couldn't be created");
		expect(primary().textContent).toContain("Create setup package");

		await next();
		await settle();
		expect(calls).toHaveLength(2);
		expect(calls[1]).toMatchObject({ name: "factory-line-9" });
		expect(text()).toContain("The package for factory-line-9 is ready.");
		expect(
			fake.hub.enrollments.filter((row) => row.name === NAME),
		).toHaveLength(0);
		expectClean();
	});
});

describe("hub checks", () => {
	test("readiness gates the verified release and the enrollment, also for a direct submit", async () => {
		const fake = await createFakeWorkspace();
		const ready = fake.hub.readiness;
		fake.hub.readiness = NOT_READY(fake);
		const { calls, settle } = await mountSetup({ fake });
		expect(stepOf()).toBe(0);
		expect(text()).toContain("can't set up devices right now");
		expect(text()).toContain("Ask your hub operator. Only they can fix this.");
		expect(text()).toContain("Open hub status");
		// The area already holds the manifest (agent updates); the setup does not take it before the hub is ready.
		expect(text()).toContain("Verified once the hub checks pass");
		expect(text()).not.toContain("Signed by");
		expect(gated()).toBe(true);
		await next();
		await submit();
		expect(stepOf()).toBe(0);
		expect(posts(fake)).toHaveLength(0);
		expectClean();

		fake.hub.readiness = ready;
		await recheck(fake);
		await settle();
		expect(text()).toContain("Signed by");
		const fingerprint = document.querySelector("[data-fingerprint]");
		expect(fingerprint?.textContent).toBeTruthy();
		expect(copyText()).not.toContain(String(fingerprint?.textContent));
		expect(gated()).toBe(false);

		await toCreateStep();
		fake.hub.readiness = NOT_READY(fake);
		await recheck(fake);
		await settle();
		expect(stepOf()).toBe(0);
		expect(gated()).toBe(true);
		await submit();
		await next();
		expect(calls).toHaveLength(0);
		expect(posts(fake)).toHaveLength(0);
	});

	test("a re-check ignores the stale readiness answer and the stale release, and gates Create while it runs", async () => {
		const fake = await createFakeWorkspace();
		const releases = holdFetch(fake.hub.release?.url ?? "");
		const waiting = () => releases.filter((held) => !held.signal?.aborted);
		const ready = fake.hub.readiness;
		const { calls, settle } = await mountSetup({ fake });
		expect(text()).toContain("Verifying the agent release…");
		expect(gated()).toBe(true);
		const checksAtMount = fake.api.sent("GET", "devices/setup").length;

		// A second check starts while the first is unanswered; the hub turns not ready before either answers.
		let letThrough = fake.api.hold({ path: "devices/setup" });
		await recheck(fake);
		expect(text()).toContain("Checking the hub…");
		await recheck(fake);
		fake.hub.readiness = NOT_READY(fake);
		letThrough();
		await settle();
		expect(fake.api.sent("GET", "devices/setup")).toHaveLength(
			checksAtMount + 2,
		);
		expect(text()).toContain("can't set up devices right now");
		expect(text()).not.toContain("Signed by");

		fake.hub.readiness = ready;
		await recheck(fake);
		await settle();
		expect(waiting().length).toBeGreaterThan(0);
		expect(text()).toContain("Verifying the agent release…");
		expect(gated()).toBe(true);

		// Checking again while the release is on its way: its answer no longer counts.
		const stale = [...releases];
		letThrough = fake.api.hold({ path: "devices/setup" });
		await recheck(fake);
		await settle();
		expect(stale.every((held) => held.signal?.aborted)).toBe(true);
		for (const held of stale) held.release();
		await settle();
		expect(text()).not.toContain("Signed by");
		expect(gated()).toBe(true);
		await submit();
		expect(stepOf()).toBe(0);

		letThrough();
		await settle();
		expect(waiting()).toHaveLength(1);
		expect(gated()).toBe(true);
		waiting()[0]?.release();
		await settle();
		expect(text()).toContain("is ready for a new device");
		expect(gated()).toBe(false);

		// At Create, a running re-check gates the button and the form alike.
		await toCreateStep();
		const before = releases.length;
		letThrough = fake.api.hold({ path: "devices/setup" });
		await recheck(fake);
		await settle();
		expect(stepOf()).toBe(4);
		expect(gated()).toBe(true);
		await next();
		await submit();
		expect(calls).toHaveLength(0);
		letThrough();
		await settle();
		expect(releases.length).toBe(before + 1);
		expect(gated()).toBe(true);
		await submit();
		expect(calls).toHaveLength(0);
		releases.at(-1)?.release();
		await settle();
		expect(gated()).toBe(false);
		await submit();
		await settle();
		expect(calls).toHaveLength(1);
		expect(posts(fake)).toHaveLength(1);
	});

	test("a failed check is retryable and shows nothing of the backend's answer", async () => {
		const fake = await createFakeWorkspace();
		const heal = fake.api.fail(
			{ path: "devices/setup" },
			new Error("private backend detail"),
		);
		const { settle } = await mountSetup({ fake });
		expect(text()).toContain("The hub checks couldn't finish.");
		expect(text()).toContain("Nothing was created.");
		expect(text()).not.toContain("private backend detail");
		expect(gated()).toBe(true);
		await next();
		expect(posts(fake)).toHaveLength(0);
		expectClean();

		heal();
		await clickByText("Check again");
		await settle();
		expect(text()).not.toContain("The hub checks couldn't finish.");
		expect(text()).toContain("is ready for a new device");
		expect(gated()).toBe(false);
	});

	test("a release that fails verification stops the setup and keeps the verifier's sentence behind Details", async () => {
		const fake = await createFakeWorkspace();
		if (!fake.hub.releaseTrust)
			throw new Error("The seed has no release trust");
		fake.hub.releaseTrust = {
			...fake.hub.releaseTrust,
			public_keys: [b64url(new Uint8Array(32).fill(7))],
		};
		const { settle } = await mountSetup({ fake });
		await settle();
		expect(text()).toContain("The agent release failed a check.");
		expect(text()).toContain(
			"The hub's agent release failed a check: it isn't signed by a key the hub operator pinned.",
		);
		expect(text()).toContain(
			"Ask your hub operator. Nothing on this computer can work around it, and nothing was created.",
		);
		expect(text()).toContain("This release can't be used");
		expect(text()).toContain("Details");
		expect(text()).toContain(
			"The package signature does not match a configured release key.",
		);
		expect(text()).not.toContain("Signed by");
		expect(gated()).toBe(true);
		await next();
		await submit();
		expect(stepOf()).toBe(0);
		expect(posts(fake)).toHaveLength(0);
	});

	const FAILED_CHECK = (reason: string) => [
		"The agent release failed a check.",
		`The reason: ${reason}. No package can be built from it, and nothing was created.`,
	];
	const REFUSED: readonly {
		name: string;
		release: TestReleaseOptions;
		reason: string;
		/** The headline's two sentences. */
		headline: readonly (string | RegExp)[];
		next: string;
		chip: string;
	}[] = [
		{
			name: "a list that has run out",
			release: { issuedAgoS: 40 * DAY_S, endsInS: -DAY_S },
			reason: "it has run out",
			headline: [
				"The hub's agent release has run out.",
				/It ran out on .+, so no package can be built until the hub operator publishes or renews a release\. Nothing was created\./,
			],
			next: "Ask your hub operator to publish or renew a release. Nothing was created.",
			chip: "Expired",
		},
		{
			name: "a list dated in the future",
			release: { issuedAgoS: -3_600 },
			reason: "it isn't valid yet; check this computer's clock",
			headline: FAILED_CHECK("it isn't valid yet; check this computer's clock"),
			next: "This can be fixed on this computer; then check again. Nothing was created.",
			chip: "Can't be verified",
		},
		{
			name: "a list valid for longer than this app accepts",
			release: { endsInS: 6 * 365 * DAY_S },
			reason: "it is valid for longer than this app accepts; update the app",
			headline: FAILED_CHECK(
				"it is valid for longer than this app accepts; update the app",
			),
			next: "This can be fixed on this computer; then check again. Nothing was created.",
			chip: "Can't be verified",
		},
	];

	for (const {
		name,
		release,
		reason,
		headline,
		next: nextStep,
		chip,
	} of REFUSED)
		test(`${name} stops the setup with its own reason`, async () => {
			const { fake } = await hubWithRelease(release);
			const { settle } = await mountSetup({ fake });
			await settle();
			const lead = String(
				document.querySelector("[data-headline]")?.textContent,
			);
			for (const sentence of headline)
				if (typeof sentence === "string") expect(lead).toContain(sentence);
				else expect(lead).toMatch(sentence);
			expect(text()).toContain(
				`The hub's agent release failed a check: ${reason}.`,
			);
			expect(text()).toContain(nextStep);
			expect(text()).toContain(
				`It failed a check: ${reason}. No package is built from it.`,
			);
			expect(text()).toContain(`Agent release${chip}`);
			expect(text()).toContain(
				"The agent release failed a check, so no package can be built.",
			);
			expect(gated()).toBe(true);
			await next();
			await submit();
			expect(stepOf()).toBe(0);
			expect(posts(fake)).toHaveLength(0);
			expectClean();
		});

	test("a release address that doesn't answer is a failed load, not a failed check", async () => {
		const { fake } = await hubWithRelease({});
		fake.hub.release = undefined;
		const { settle } = await mountSetup({ fake });
		await settle();
		expect(text()).toContain("The agent release couldn't be loaded");
		expect(text()).not.toContain("failed a check");
		expect(gated()).toBe(true);
	});

	test("a list valid for a year passes and shows its end without a day count", async () => {
		const { fake, published } = await hubWithRelease({});
		await mountSetup({ fake });
		expect(text()).toContain("All 6 hub checks pass, agent 0.9.4 is verified");
		expect(text()).toContain(
			`valid until ${moment(fake, published.manifest.expires_at)}`,
		);
		expect(text()).toContain("release number 44");
		expect(text()).not.toMatch(/days? left/);
		expect(text()).not.toContain("runs out");
		expect(text()).not.toContain("30 days");
		expect(gated()).toBe(false);
		expectClean();
	});

	test("inside the last 30 days the check says until when setup works", async () => {
		const { fake } = await hubWithRelease({ endsInS: 12 * DAY_S });
		await mountSetup({ fake });
		expect(text()).toMatch(/valid until .+ \(12 days left\)/);
		expect(text()).toMatch(
			/This hub's agent release runs out on .+\. Setup works until then\./,
		);
		// A day's package still fits: the usual time to start it stays.
		expect(text()).toContain("24 h to start the package once it's made.");
		expect(gated()).toBe(false);
		expectClean();
	});

	test("a release that ends before the package would is the time to start by, on every step", async () => {
		const { fake, published } = await hubWithRelease({ endsInS: 5 * 3_600 });
		const end = moment(fake, published.manifest.expires_at);
		const startBy = `before ${end}: the hub's agent release runs out then.`;
		const { settle } = await mountSetup({ fake });
		expect(text()).toContain(
			`A package made now has to be started on the device ${startBy}`,
		);
		expect(text()).toContain(
			`About 10 minutes. A package made now has to be started on the device ${startBy}`,
		);
		expect(text()).not.toContain("24 h to start the package");
		expect(text()).toContain(`valid until ${end} (less than a day left)`);

		await toCreateStep();
		expect(text()).toContain(
			`Usually under a minute. A package made now has to be started on the device ${startBy}`,
		);
		expect(text()).not.toContain("The package then works for");
		await next();
		await settle();
		expect(text()).toContain(
			`factory-line-4 is registered and waits for its first start until ${end}.`,
		);
		expect(storedDrafts(fake)).toContain(
			`"releaseEndsAt":${published.manifest.expires_at}`,
		);
		expect(trayItems(fake)[0]?.deadlineAt).toBe(
			published.manifest.expires_at * 1000,
		);

		await next();
		expect(stepOf()).toBe(5);
		expect(text()).toContain(
			`The package for factory-line-4 works until ${end}.`,
		);
		expect(text()).toContain(`Start it on the device ${startBy}`);
		expect(text()).toContain("then the hub's agent release runs out");
		expect(text()).toContain(`works until ${end}`);

		const download = byRole("link", /Download setup package/);
		download.addEventListener("click", (event) => event.preventDefault());
		await click(download);
		await next();
		await next();
		await settle();
		expect(stepOf()).toBe(7);
		expect(text()).toContain(`Start it on the device ${startBy}`);
		expectClean();
	});

	test("a package whose agent release ran out can't be started, also after a reload, and its setup can still be cancelled", async () => {
		const { fake, published } = await hubWithRelease({ endsInS: 5 * 3_600 });
		const first = await mountSetup({ fake });
		await create(first.settle);
		await next();
		expect(stepOf()).toBe(5);
		await first.unmount();

		// An hour after the release in the package ran out; the setup at the hub has 18 hours left.
		const reloaded = await mountSetup({
			search: "flow=setup&step=5",
			now: (published.manifest.expires_at + 3_600) * 1000,
		});
		expect(stepOf()).toBe(7);
		expect(text()).toContain(
			`The package for factory-line-4 expired unused on ${moment(reloaded.fake, published.manifest.expires_at)}.`,
		);
		expect(primary().textContent).toContain("Create a new one");
		expect(byRole("button", /Cancel setup/)).toBeTruthy();
		expect(reloaded.calls).toHaveLength(0);
	});

	test("a release that runs out while the wizard is open stops the setup", async () => {
		const { fake } = await hubWithRelease({ endsInS: 2 * 3_600 });
		const { settle, unmount } = await mountSetup({ fake, tickMs: 100 });
		expect(gated()).toBe(false);
		fake.clock.advance(3 * 3_600_000);
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 300));
		});
		expect(text()).toContain(
			"The hub's agent release failed a check: it has run out.",
		);
		expect(gated()).toBe(true);
		await next();
		await settle();
		expect(stepOf()).toBe(0);
		expect(posts(fake)).toHaveLength(0);
		// The clock is live here: stop it before the test hands back to the runner.
		await unmount();
	});

	test("large agents stay selectable with the first-start download explained; Intel Mac and Docker on a Mac are disabled with reasons", async () => {
		const fake = await largeAgentHub();
		const { calls, settle } = await mountSetup({ fake });
		await next();
		await typeInto(byRole("textbox", /Name this device/), NAME);
		await next();
		expect(stepOf()).toBe(2);

		const linux = byRole("radio", /Linux \(Intel\/AMD 64-bit\)/);
		const macArm = byRole("radio", /Mac \(Apple silicon\)/);
		const macIntel = byRole("radio", /Mac \(Intel\)/);
		expect(linux.hasAttribute("disabled")).toBe(false);
		expect(macArm.hasAttribute("disabled")).toBe(false);
		expect(macIntel.hasAttribute("disabled")).toBe(true);
		expect(text()).toContain("This release has no package for Mac (Intel).");
		expect(text()).toContain(
			"Downloads on first start (larger than 256 MiB). Needs internet, curl and shasum on the device.",
		);

		await click(linux);
		await click(byRole("radio", /Docker Compose/));
		expect(byRole("radio", /^Both/).hasAttribute("disabled")).toBe(false);
		await click(macArm);
		expect(byRole("radio", /Run directly/).getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(byRole("radio", /Docker Compose/).hasAttribute("disabled")).toBe(
			true,
		);
		expect(text()).toContain("Docker packages exist only for Linux.");
		expect(text()).toContain("Switched to Run directly");
		expectClean();

		await next();
		await enterPassword();
		expect(text()).toContain("the agent downloads on first start");
		await next();
		await settle();
		expect(calls[0]).toMatchObject({
			mode: "binary",
			target: "aarch64-apple-darwin",
		});
		expect(text()).toContain("Downloading the agent · skipped");
		expect(text()).toContain("The device downloads it on first start");
	});

	test("the unused-package limit and the daily cap stop the setup before anything is typed", async () => {
		const full = await createFakeWorkspace();
		full.hub.limits.max_pending_enrollments = 1;
		const first = await mountSetup({ fake: full });
		expect(text()).toContain(
			"Too many setups are waiting to be started (1 of 1).",
		);
		expect(text()).toContain("Cancel one in Pending setups to continue");
		expect(text()).toContain("Open Pending setups");
		expect(gated()).toBe(true);
		const sent = full.api.calls.length;
		await next();
		await submit();
		expect(stepOf()).toBe(0);
		expect(full.api.calls).toHaveLength(sent);
		expectClean();
		await first.unmount();

		const capped = await createFakeWorkspace();
		capped.hub.limits.max_enrollments_per_day =
			capped.seed.usage?.usage.enrollments_last_24h ?? 1;
		await mountSetup({ fake: capped });
		expect(text()).toContain("You've reached today's setup limit");
		expect(text()).toContain(
			"The limit counts packages from the last 24 hours",
		);
		expect(gated()).toBe(true);
		await next();
		expect(stepOf()).toBe(0);
		expect(posts(capped)).toHaveLength(0);
		expectClean();
	});

	test("a hub that refuses the enrollment for its limits says so without the raw error", async () => {
		const fake = await createFakeWorkspace();
		const { settle } = await mountSetup({ fake });
		await toCreateStep();
		fake.hub.limits.max_pending_enrollments = 1;
		await next();
		await settle();
		expect(text()).toContain("couldn't be created");
		expect(text()).toContain("The hub refused another setup");
		expect(text()).toContain("Pending device enrollment limit reached");
		expect(text()).not.toContain("HTTP_429");
		expect(text()).not.toContain("TOO_MANY_REQUESTS");
		expect(primary().textContent).toContain("Try again");
		expect(storedDrafts(fake)).not.toContain("created");
	});
});

describe("a pending setup", () => {
	test("opens at the start instructions with the earlier steps locked", async () => {
		const { fake, calls, settle } = await mountSetup({
			search: `flow=setup&enrollment=${PENDING}&step=6`,
		});
		expect(stepOf()).toBe(6);
		expect(text()).toContain("factory-line-3 is waiting to be started.");
		expect(text()).toContain("Only the steps from here on apply.");
		expect(text()).toContain("unzip flow-like-factory-line-3.zip");
		expect(text()).toContain(
			"This window doesn't know how the package was built",
		);
		const back = byRole("button", "Back");
		expect(back.getAttribute("aria-disabled")).toBe("true");
		expect(text()).toContain("Earlier steps are locked: the package is built.");
		await click(back);
		expect(stepOf()).toBe(6);
		expectClean();

		await clickByText("Docker Compose");
		expect(text()).toContain("sh start-docker.sh");
		await next();
		await settle();
		expect(stepOf()).toBe(7);
		expect(text()).toContain("Waiting for factory-line-3 to start.");
		expect(calls).toHaveLength(0);
		expect(setupWrites(fake)).toHaveLength(0);
	});

	test("a link to an earlier step lands on the instructions and says why", async () => {
		await mountSetup({ search: `flow=setup&enrollment=${PENDING}&step=2` });
		expect(stepOf()).toBe(6);
		expect(text()).toContain("The earlier steps are locked.");
	});

	test("that lapsed shows the expired state and offers a new setup under the same name", async () => {
		const { fake, navigations, settle } = await mountSetup({
			search: `flow=setup&enrollment=${LAPSED}`,
		});
		expect(stepOf()).toBe(7);
		expect(text()).toContain("The package for test-vm expired unused on");
		expect(text()).toContain("The package expired unused");
		expect(primary().textContent).toContain("Create a new one");
		expect(queryByRole("button", /Cancel setup/)).toBeNull();
		expectClean();

		await next();
		await settle();
		expect(navigations.at(-1)?.href).toBe("/settings/devices?flow=setup");
		expect(stepOf()).toBe(0);
		await next();
		expect(
			(byRole("textbox", /Name this device/) as HTMLInputElement).value,
		).toBe("test-vm");
		expect(setupWrites(fake)).toHaveLength(0);
	});

	test("made in this window is offered again when a new setup starts", async () => {
		const first = await mountSetup();
		await create(first.settle);
		await next();
		const enrollment = first.fake.hub.enrollments.find(
			(row) => row.name === NAME,
		);
		await first.unmount();

		const again = await mountSetup();
		expect(stepOf()).toBe(0);
		expect(text()).toContain("factory-line-4 is still waiting to be started.");
		const link = byRole("link", /Continue that setup/);
		expect(link.getAttribute("href")).toBe(
			`/settings/devices?flow=setup&enrollment=${enrollment?.enrollment_id}&step=5`,
		);
		await clickByText("Dismiss");
		expect(text()).not.toContain("is still waiting to be started");
		expect(again.calls).toHaveLength(0);
	});
});

describe("older hubs and agents", () => {
	test("a hub without the usage and pending-setup routes shows limits without usage and the setups this computer tracks", async () => {
		const { fake, settle } = await mountSetup({ hubVersion: "old" });
		await settle();
		expect(text()).toContain("is ready for a new device");
		expect(text()).toContain(
			"All 6 hub checks pass and agent 0.9.4 is verified.",
		);
		expect(text()).toContain("tracked on this computer");
		expect(text()).not.toContain("Couldn't load");
		expect(allByRole("alert")).toHaveLength(0);
		expect(gated()).toBe(false);
		for (const route of ["devices/usage", "devices/enrollments"])
			expect(fake.api.sent("GET", route).length).toBeLessThanOrEqual(1);
		expectClean();

		await toCreateStep();
		expect(text()).toContain(
			"Cancel the setup any time before the device starts it. That frees the slot again.",
		);
		expect(text()).not.toContain("unused-package slots (");
		await next();
		await settle();
		// The setup made here is this computer's own record of it (BG2 interim).
		expect(text()).toContain("Package ready");
		const pending = byRole("region", /Pending setups/);
		expect(pending.textContent).toContain("factory-line-4");
		expect(pending.textContent).toContain("this window");
		expect(pending.textContent).toContain(
			"Unused packages count toward your limit until they're used, cancelled or expire.",
		);
		await next();
		await clickByText("Cancel setup…");
		expect(inPortal().textContent).toContain("Frees its unused-package slot.");
		for (const route of ["devices/usage", "devices/enrollments"])
			expect(fake.api.sent("GET", route).length).toBeLessThanOrEqual(2);
	});

	test("an older agent changes nothing: the wizard sends no device command", async () => {
		const { fake, settle } = await mountSetup({ agentFeatures: {} });
		const before = fake.api.commands.length;
		await create(settle);
		await next();
		expect(fake.api.commands).toHaveLength(before);
		expect(allByRole("alert")).toHaveLength(0);
		expect(text()).not.toContain("Couldn't load");
	});
});

describe("the activity tray and this computer", () => {
	test("a setup is tracked from its registration on, and a cancelled one leaves no entry", async () => {
		const { fake, settle } = await mountSetup();
		expect(trayItems(fake)).toHaveLength(0);
		await create(settle);
		expect(trayItems(fake)).toMatchObject([
			{
				state: "waiting",
				target: { deviceName: NAME },
				resume: { type: "setup" },
			},
		]);
		await next();
		await clickByText("Cancel setup…");
		await clickByText("Cancel setup for factory-line-4", inPortal());
		await settle();
		expect(trayItems(fake)).toHaveLength(0);
	});

	test("a browser that may delete keys says so before anything is made, and can be asked to keep them", async () => {
		const { fake, settle } = await mountSetup({
			platform: "web",
			persistence: "denied",
		});
		expect(text()).toContain("Browser may delete keys");
		expect(text()).toContain(
			"Keep the account backup on and save the key backup file in the Save step.",
		);
		expect(gated()).toBe(false);
		fake.browser.persistence = "persisted";
		await clickByText("Keep keys safely");
		await settle();
		expect(text()).toContain("The browser granted persistent storage at");
		expect(text()).toContain("Web · kept safely");
		expectClean();
	});

	test("the review step leads back to each choice", async () => {
		await mountSetup();
		await toCreateStep();
		await click(byRole("button", "Change platform"));
		expect(stepOf()).toBe(2);
		expect(
			byRole("radio", /Linux \(Intel\/AMD 64-bit\)/).getAttribute(
				"aria-checked",
			),
		).toBe("true");
	});
});
