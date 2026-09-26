import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { IProfile } from "../../../types";
import type { OpenFleet } from "../../../lib/device-management/fleet";
import type {
	BrowserController,
	PlacementStatus,
} from "../../../lib/device-management/types";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	HTMLInputElement: window.HTMLInputElement,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const originalSetInterval = globalThis.setInterval;
const originalClearInterval = globalThis.clearInterval;
const timers = new Map<number, () => void>();
let timerId = 0;
Object.defineProperty(globalThis, "setInterval", {
	configurable: true,
	value: (callback: () => void, delay: number) => {
		if (delay !== 30_000) throw new Error("Unexpected fleet polling interval");
		const id = ++timerId;
		timers.set(id, callback);
		return id;
	},
});
Object.defineProperty(globalThis, "clearInterval", {
	configurable: true,
	value: (id: number) => timers.delete(id),
});
const scope = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const profile = {} as IProfile;
let reports: [string, OpenFleet | undefined][] = [];
let reads: string[] = [];
let registrations: string[] = [];
let calls: string[] = [];
let releases: string[] = [];
let controllers: {
	id: string;
	close: ReturnType<typeof mock>;
	free: ReturnType<typeof mock>;
}[] = [];
let responses = new Map<string, Promise<OpenFleet>>();
let lockError: Error | undefined;
const backend = {
	apiState: {
		get: async (_profile: unknown, path: string) => {
			calls.push(path);
			if (!path.endsWith("/identity"))
				throw new Error("No management connection is allowed");
			return {};
		},
	},
};
const report = (device: string, snapshot: OpenFleet | undefined) =>
	reports.push([device, snapshot]);
function snapshot(id: string): OpenFleet {
	const row = {
		id: `placement-${id}`,
		project_id: "project",
		deployment_id: "deployment",
		revision: `revision-${id}`,
		desired_state: "running",
		observed_state: "running",
		config_revision: 1,
		intent_revision: 1,
		ready_replicas: 1,
		desired_replicas: 1,
	} as PlacementStatus;
	return {
		observations: [
			{
				scope: { kind: "device" },
				device_id: id,
				boot_id: "boot",
				observed_at: 1000,
				placements: [row],
			},
		],
		metrics: [],
	};
}
function deferred<T>() {
	let resolve!: (value: T) => void, reject!: (error: Error) => void;
	const promise = new Promise<T>((ok, fail) => {
		resolve = ok;
		reject = fail;
	});
	return { promise, resolve, reject };
}
mock.module("../../../state/backend-state", () => ({
	useBackend: () => backend,
}));
mock.module("../../../lib/device-management/storage", () => ({
	acquireDeviceLock: async (_scope: unknown, id: string) => {
		if (lockError) throw lockError;
		return () => releases.push(id);
	},
	readDeviceVault: async (_scope: unknown, id: string) => ({
		deviceId: id,
		controllerVault: new Uint8Array(64),
	}),
}));
mock.module("../../../lib/device-management/crypto", () => ({
	withPassword: async (
		password: string,
		run: (value: Uint8Array) => unknown,
	) => {
		expect(password).toBe("device password");
		return run(new TextEncoder().encode(password));
	},
	loadDeviceCrypto: async () => ({
		unlockControllerVault: (id: string) => {
			const controller = { id, close: mock(), free: mock() };
			controllers.push(controller);
			return controller;
		},
	}),
}));
mock.module("../../../lib/device-management/fleet", () => ({
	registerFleetReader: async (
		_api: unknown,
		_profile: unknown,
		_scope: unknown,
		controller: BrowserController & { id: string },
	) => {
		registrations.push(controller.id);
	},
	readFleet: async (
		_api: unknown,
		_profile: unknown,
		_scope: unknown,
		controller: BrowserController & { id: string },
	) => {
		reads.push(controller.id);
		return responses.get(controller.id) ?? snapshot(controller.id);
	},
}));
mock.module("../../../lib/device-management/inventory", () => ({
	visibleInventory: (
		observations: OpenFleet["observations"],
		project?: string,
	) =>
		observations.flatMap((observation) =>
			observation.placements
				.filter((row) => !project || row.project_id === project)
				.map((row) => ({ row, at: observation.observed_at })),
		),
}));
mock.module("./device-metrics-view", () => ({
	DeviceMetricsView: () => <div>metrics</div>,
}));
const { createRoot } = await import("react-dom/client");
const { DeviceFleetMonitor } = await import("./device-fleet-monitor");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
beforeEach(() => {
	reports = [];
	reads = [];
	registrations = [];
	calls = [];
	releases = [];
	controllers = [];
	responses = new Map();
	lockError = undefined;
});
afterEach(async () => {
	await act(async () => root.render(null));
	timers.clear();
});
afterAll(async () => {
	await act(async () => root.unmount());
	globalThis.setInterval = originalSetInterval;
	globalThis.clearInterval = originalClearInterval;
	mock.restore();
	await window.happyDOM.close();
});
async function render(ids = ["one"], project: string | null = "project") {
	await act(async () =>
		root.render(
			<>
				{ids.map((id) => (
					<section key={id} data-device={id}>
						<DeviceFleetMonitor
							deviceId={id}
							profile={profile}
							scope={scope}
							projectId={project ?? undefined}
							onSnapshot={report}
						/>
					</section>
				))}
			</>,
		),
	);
}
function panel(id = "one") {
	const value = container.querySelector<HTMLElement>(`[data-device=${id}]`);
	if (!value) throw new Error("Missing panel");
	return value;
}
async function click(label: string, id = "one") {
	await act(async () => {
		const value = [...panel(id).querySelectorAll("button")].find(
			(button) => button.textContent === label,
		);
		if (!value)
			throw new Error(`Missing button ${label}: ${panel(id).textContent}`);
		value.click();
	});
}
async function unlock(id = "one", open = true) {
	if (open) await click("Fleet inventory and metrics", id);
	await act(async () => {
		const input = panel(id).querySelector<HTMLInputElement>(
			"input[type=password]",
		);
		if (!input) throw new Error("Missing password");
		input.value = "device password";
		panel(id)
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
	});
}
async function tick() {
	await act(async () => {
		for (const timer of [...timers.values()]) timer();
		for (let i = 0; i < 8; i++) await Promise.resolve();
	});
}

test("unlocked fleet panels poll retained snapshots without a management or Noise connection", async () => {
	await render();
	await unlock();
	expect(registrations).toEqual(["one"]);
	expect(reads).toEqual(["one"]);
	expect(calls).toEqual(["devices/one/identity"]);
	expect(panel().textContent).toContain("revision-one");
	expect(panel().querySelector("input[type=password]")).toBeNull();
	await tick();
	expect(reads).toEqual(["one", "one"]);
	expect(calls).toEqual(["devices/one/identity"]);
});

test("a failed device lock clears the submitted password before key loading", async () => {
	lockError = new Error("Device is already unlocked in another tab");
	await render();
	await unlock();
	expect(panel().querySelector<HTMLInputElement>("input[type=password]")?.value).toBe("");
	expect(panel().querySelector("[role=alert]")?.textContent).toContain("already unlocked");
	expect(controllers).toEqual([]);
	expect(registrations).toEqual([]);
	expect(reads).toEqual([]);
});

test("locking and unmounting discard late decrypted callbacks and release the controller", async () => {
	const pending = deferred<OpenFleet>();
	responses.set("one", pending.promise);
	await render();
	await unlock();
	await click("Close monitoring");
	expect(controllers[0].close).toHaveBeenCalledTimes(1);
	expect(controllers[0].free).toHaveBeenCalledTimes(1);
	await act(async () => {
		pending.resolve(snapshot("one"));
		await pending.promise;
	});
	expect(reports.filter(([, value]) => value)).toEqual([]);
	expect(container.textContent).not.toContain("revision-one");
	const after = deferred<OpenFleet>();
	responses.set("one", after.promise);
	await unlock();
	await act(async () => root.render(null));
	await act(async () => {
		after.resolve(snapshot("one"));
		await after.promise;
	});
	expect(reports.filter(([, value]) => value)).toEqual([]);
	expect(releases).toEqual(["one", "one"]);
});

test("independent device panels retain only their own decrypted snapshots", async () => {
	await render(["one", "two"]);
	await unlock("one");
	await unlock("two");
	expect(panel("one").textContent).toContain("revision-one");
	expect(panel("one").textContent).not.toContain("revision-two");
	expect(panel("two").textContent).toContain("revision-two");
	await click("Lock monitoring", "one");
	expect(panel("one").textContent).not.toContain("revision-one");
	expect(panel("two").textContent).toContain("revision-two");
	await tick();
	expect(reads).toEqual(["one", "two", "two"]);
	expect(reports.filter(([, value]) => value).map(([id]) => id)).toEqual([
		"one",
		"two",
		"two",
	]);
});

test("a failed current poll clears stale decrypted inventory and requires unlocking again", async () => {
	await render();
	await unlock();
	const failed = deferred<OpenFleet>();
	responses.set("one", failed.promise);
	await tick();
	await act(async () => {
		failed.reject(new Error("Access revoked"));
		await failed.promise.catch(() => undefined);
	});
	expect(panel().textContent).not.toContain("revision-one");
	expect(panel().querySelector('[role="alert"]')?.textContent).toContain(
		"Access revoked",
	);
	expect(panel().querySelector("input[type=password]")).not.toBeNull();
	expect(reports.at(-1)).toEqual(["one", undefined]);
});

test("a failed old poll cannot lock a newly unlocked controller or suppress its first refresh", async () => {
	await render();
	await unlock();
	const old = deferred<OpenFleet>();
	responses.set("one", old.promise);
	await tick();
	await click("Lock monitoring");
	responses.delete("one");
	await unlock("one", false);
	expect(reads).toEqual(["one", "one", "one"]);
	const replacement = controllers[1];
	await act(async () => {
		old.reject(new Error("old cancelled poll"));
		await old.promise.catch(() => undefined);
	});
	expect(replacement.close).toHaveBeenCalledTimes(0);
	expect(replacement.free).toHaveBeenCalledTimes(0);
	expect(panel().textContent).toContain("revision-one");
	expect(panel().querySelector('[role="alert"]')).toBeNull();
});

test("metrics-only readers are not shown an empty inventory claim and every metric scope is named", async () => {
	responses.set(
		"one",
		Promise.resolve({
			observations: [],
			metrics: [
				{ scope: { kind: "device" }, observedAt: 1, sample: {} },
				{
					scope: { kind: "project", project_id: "project" },
					observedAt: 1,
					sample: {},
				},
				{
					scope: {
						kind: "placement",
						project_id: "project",
						placement_id: "api",
					},
					observedAt: 1,
					sample: {},
				},
			],
		}),
	);
	await render(["one"], null);
	await unlock();
	expect(panel().textContent).toContain(
		"No authorized inventory snapshot is available yet.",
	);
	expect(panel().textContent).not.toContain(
		"No placements in the latest authorized inventory.",
	);
	for (const label of [
		"Device metrics",
		"Project metrics: project",
		"Placement metrics: project / api",
		"overlapping scopes",
	])
		expect(panel().textContent).toContain(label);
});

test("an authorized empty status snapshot reports empty placements", async () => {
	const empty = snapshot("one");
	empty.observations[0].placements = [];
	responses.set("one", Promise.resolve(empty));
	await render();
	await unlock();
	expect(panel().textContent).toContain(
		"No placements in the latest authorized inventory.",
	);
	expect(panel().textContent).not.toContain(
		"No authorized inventory snapshot is available yet.",
	);
});
