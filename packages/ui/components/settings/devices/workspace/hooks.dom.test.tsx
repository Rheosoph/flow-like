import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { AppView } from "../../../../lib/device-management/model/app-plan";
import type { DeviceViewModel } from "../../../../lib/device-management/model/types";
import type { StreamState } from "../../../../lib/device-management/workspace/types";
import { installDom } from "../testing/dom-harness";
import type * as Hooks from "./index";
import type * as Harness from "./test-harness";

const dom = installDom();
const harness = await import("./test-harness");
const hooks = await import("./index");

const { Capture, TestProviders, createTestWorkspace, testPlacement, testRow } =
	harness;
const NOW_S = harness.TEST_NOW_MS / 1000;
const restoreBackend = harness.installTestBackend(undefined, {
	appState: {
		getApp: async (appId: string) => ({ id: appId, visibility: "Private" }),
		getAppMeta: async () => ({ name: "Support Portal" }),
	},
	eventState: {
		getEvents: async () => [
			{
				id: "event-1",
				name: "Chat",
				active: true,
				event_type: "simple_chat",
				event_version: [1, 0, 0],
				board_version: [1, 0, 0],
			},
		],
	},
});

afterEach(dom.cleanup);
afterAll(() => {
	restoreBackend();
	dom.restore();
});

type Captured<T> = Partial<Harness.Captured<T>>;

describe("device and service views", () => {
	test("a locked device says why its services can't be read; unlocked and connected they are live rows", async () => {
		const test = createTestWorkspace();
		const device: Captured<DeviceViewModel | undefined> = {};
		const service: Captured<Hooks.ServiceViewRead> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useDeviceView("dev-1")} into={device} />
				<Capture
					read={() => hooks.useServiceView("dev-1", "svc-1")}
					into={service}
				/>
			</TestProviders>,
		);
		await test.idle();
		expect(device.current).toMatchObject({
			presence: { kind: "online" },
			relationship: "owner",
			keys: { state: "locked" },
			live: { kind: "idle" },
			services: { state: "locked" },
		});
		expect(service.current).toMatchObject({
			service: undefined,
			unavailable: { state: "locked" },
		});

		await test.unlock("dev-1");
		await test.idle();
		const services = device.current?.services;
		expect(
			Array.isArray(services) && services.map((row) => row.serviceId),
		).toEqual(["svc-1"]);
		expect(service.current?.service).toMatchObject({
			serviceId: "svc-1",
			conv: "converged",
			instances: { requested: 2, ready: 2, max: 4 },
			freshness: { src: "live", age: "live" },
		});
		expect(service.current?.unavailable).toBeUndefined();
	});

	test("a mounted device view watches its encrypted status and stops when it unmounts", async () => {
		const test = createTestWorkspace();
		const watched: string[] = [];
		const watch = test.workspace.fleet.watch.bind(test.workspace.fleet);
		test.workspace.fleet.watch = (deviceId) => {
			watched.push(`+${deviceId}`);
			const release = watch(deviceId);
			return () => {
				watched.push(`-${deviceId}`);
				release();
			};
		};
		const view = await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useDeviceView("dev-1")} into={{}} />
				<Capture
					read={() => hooks.useDeviceView("dev-1", { watch: false })}
					into={{}}
				/>
			</TestProviders>,
		);
		await test.idle();
		expect(watched).toEqual(["+dev-1"]);
		await view.unmount();
		expect(watched).toEqual(["+dev-1", "-dev-1"]);
	});

	test("list views and coverage come from the one computation, without a watch per row", async () => {
		const test = createTestWorkspace({
			rows: [
				testRow("dev-1"),
				testRow("dev-2"),
				testRow("dev-3", { status: "revoked" }),
			],
			vaults: ["dev-1"],
		});
		const views: Captured<DeviceViewModel[]> = {};
		const coverage: Captured<Hooks.Coverage> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useDeviceViews()} into={views} />
				<Capture read={() => hooks.useCoverage()} into={coverage} />
			</TestProviders>,
		);
		await test.idle();
		expect(views.current?.map((view) => view.row.device_id)).toEqual([
			"dev-1",
			"dev-2",
			"dev-3",
		]);
		expect(views.current?.map((view) => view.health)).toContain("revoked");
		expect(coverage.current).toMatchObject({
			total: 2,
			readable: 0,
			locked: ["dev-1"],
			noKeys: ["dev-2"],
		});
		await test.unlock("dev-1");
		await test.idle();
		expect(coverage.current).toMatchObject({
			readable: 1,
			live: 1,
			locked: [],
		});
	});
});

describe("keys", () => {
	test("the key chip counts open sessions and Lock all closes them", async () => {
		const test = createTestWorkspace({
			rows: [testRow("dev-1"), testRow("dev-2")],
		});
		const chip: Captured<Hooks.KeyChip> = {};
		const local: Captured<ReturnType<typeof Hooks.useLocalSummary>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={hooks.useKeyChip} into={chip} />
				<Capture read={hooks.useLocalSummary} into={local} />
			</TestProviders>,
		);
		await test.idle();
		expect(chip.current?.unlockedCount).toBe(0);
		expect(chip.current?.sessions.map((row) => row.deviceId)).toEqual([
			"dev-1",
			"dev-2",
		]);
		expect(local.current?.vaults.length).toBe(2);
		expect(local.current?.persistence).toBe("persisted");

		await test.unlock("dev-1", false);
		await test.unlock("dev-2", false);
		await test.idle();
		expect(chip.current?.unlockedCount).toBe(2);
		await act(async () => chip.current?.lockAll());
		await test.idle();
		expect(chip.current?.unlockedCount).toBe(0);
		expect(test.workspace.keys.controller("dev-1")).toBeUndefined();
	});

	test("pre-flight runs without keys and re-runs when the key session changes", async () => {
		const test = createTestWorkspace();
		const read: Captured<Hooks.PreflightRead> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.usePreflight("dev-1")} into={read} />
			</TestProviders>,
		);
		await test.idle();
		const first = read.current?.preflight;
		expect(first?.rows.map((row) => row.id).slice(0, 9)).toEqual([
			"D1",
			"D2",
			"D3",
			"D4",
			"D5",
			"D6",
			"D7",
			"D8",
			"D9",
		]);
		const codes = Object.fromEntries(
			(first?.rows ?? []).map((row) => [row.id, row.copy.code]),
		);
		expect(codes).toMatchObject({
			D1: "hub_ready",
			D2: "signed_in",
			D3: "access_owner",
			D4: "keys_owner_here",
			D8: "checkin_online",
		});
		expect(read.current?.loading).toBe(false);

		await test.unlock("dev-1", false);
		await test.idle();
		expect(read.current?.preflight).not.toBe(first);
	});
});

describe("live", () => {
	test("demand opens a session while mounted; the hook reports state, steps and services", async () => {
		const test = createTestWorkspace();
		await test.unlock("dev-1", false);
		const session: Captured<Hooks.LiveSessionView> = {};
		const view = await dom.render(
			<TestProviders test={test}>
				<Capture
					read={() => hooks.useLiveSession("dev-1", { demand: true })}
					into={session}
				/>
			</TestProviders>,
		);
		await test.idle();
		await act(async () => session.current?.refresh());
		expect(session.current?.state.kind).toBe("live");
		expect(session.current?.steps.map((step) => step.id)).toContain(
			"getting_pass",
		);
		expect(session.current?.inspection?.value.device_id).toBe("dev-1");
		expect((test.devices.get("dev-1") as Harness.FakeDevice).connects).toBe(1);
		await view.unmount();
	});

	test("without demand the hook only observes: nothing connects", async () => {
		const test = createTestWorkspace();
		await test.unlock("dev-1", false);
		const session: Captured<Hooks.LiveSessionView> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useLiveSession("dev-1")} into={session} />
			</TestProviders>,
		);
		await test.idle();
		expect(session.current?.state).toEqual({ kind: "idle" });
		expect((test.devices.get("dev-1") as Harness.FakeDevice).connects).toBe(0);
	});

	test("a stream reports 'not loaded' until it has read, and never subscribes for a null spec", async () => {
		const test = createTestWorkspace();
		const subscribed: string[] = [];
		const subscribe = test.workspace.streams.subscribe.bind(
			test.workspace.streams,
		);
		test.workspace.streams.subscribe = ((deviceId, spec, listener) => {
			subscribed.push(spec.kind);
			return subscribe(deviceId, spec, listener);
		}) as typeof test.workspace.streams.subscribe;
		const idle: Captured<StreamState<unknown>> = {};
		const stream: Captured<Hooks.LiveStreamView<unknown>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useLiveStream("dev-1", null)} into={idle} />
				<Capture
					read={() => hooks.useLiveStream("dev-1", { kind: "certificates" })}
					into={stream}
				/>
			</TestProviders>,
		);
		await test.idle();
		expect(idle.current).toMatchObject({
			freshness: { src: "live", age: "notloaded" },
			gaps: [],
		});
		expect(subscribed).toEqual(["certificates"]);
		expect(stream.current?.started).toBe(true);
		expect(stream.current?.freshness.src).toBe("live");
	});
});

describe("activity", () => {
	test("the tray groups in progress, no reply and finished; a filter narrows to one device", async () => {
		const test = createTestWorkspace({
			rows: [testRow("dev-1"), testRow("dev-2")],
		});
		const all: Captured<Hooks.ActivityView> = {};
		const one: Captured<Hooks.ActivityView> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useActivity()} into={all} />
				<Capture
					read={() => hooks.useActivity({ deviceId: "dev-2" })}
					into={one}
				/>
			</TestProviders>,
		);
		await test.idle();
		const start = (deviceId: string) =>
			test.workspace.activity.start({
				kind: "command",
				target: { deviceId },
				state: "active",
				label: { code: "command" },
				startedBy: "you",
				actions: [],
			});
		let done = "";
		let lost = "";
		await act(async () => {
			start("dev-1");
			done = start("dev-1");
			lost = start("dev-2");
			test.workspace.activity.finish(done, "done");
			test.workspace.activity.finish(lost, "unknown", { code: "no_reply" });
		});
		expect(all.current?.inProgress.length).toBe(1);
		expect(all.current?.finished.map((item) => item.id)).toEqual([done]);
		expect(all.current?.noReply.map((item) => item.id)).toEqual([lost]);
		expect(one.current?.items.map((item) => item.id)).toEqual([lost]);

		await act(async () => all.current?.dismiss(done));
		expect(all.current?.finished).toEqual([]);
	});
});

describe("app view", () => {
	test("App › Devices: metadata, events, devices and the older-hub placements interim", async () => {
		const test = createTestWorkspace({
			rows: [testRow("dev-1"), testRow("dev-2", { last_seen_at: NOW_S - 30 })],
			vaults: ["dev-1"],
			placements: {
				"dev-1": [testPlacement("svc-1", { project_id: "app-1" })],
			},
		});
		await test.unlock("dev-1");
		const read: Captured<Hooks.AppViewRead> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useAppView("app-1")} into={read} />
			</TestProviders>,
		);
		await test.idle();
		expect(read.current?.loading).toBe(false);
		expect(read.current?.placements.missingOnHub).toBe(true);
		expect(read.current?.app).toMatchObject({
			id: "app-1",
			name: "Support Portal",
			visibility: "Private",
			events: [{ id: "event-1", name: "Chat" }],
		});
		const view = read.current?.view as AppView;
		expect(view.app).toMatchObject({ mode: "online", canReadFlows: true });
		expect(view.coverage).toMatchObject({ total: 2, readable: 1 });
		expect(view.services.map((row) => row.serviceId)).toEqual(["svc-1"]);
	});

	test("without an app id nothing is read", async () => {
		const test = createTestWorkspace();
		const read: Captured<Hooks.AppViewRead> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useAppView(undefined)} into={read} />
			</TestProviders>,
		);
		await test.idle();
		expect(read.current).toMatchObject({ view: undefined, loading: false });
		expect(
			test.hub.calls.some(([, path]) => path.includes("device-placements")),
		).toBe(false);
	});
});
