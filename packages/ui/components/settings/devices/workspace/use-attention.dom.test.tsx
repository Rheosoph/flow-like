import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { HubDeviceSupport } from "../../../../lib/device-management/model/types";
import { advance, installDom } from "../testing/dom-harness";
import type * as Hooks from "./index";
import type * as Harness from "./test-harness";

const dom = installDom();
const harness = await import("./test-harness");
const hooks = await import("./index");
const { deviceKeys } = await import(
	"../../../../lib/device-management/hub/queries"
);
const { evaluateGate } = await import(
	"../../../../lib/device-management/model/gates"
);
const { useAreaNow } = await import("../primitives/area-context");

const { Capture, TestProviders, createTestWorkspace, testRow, TEST_ME } =
	harness;
const NOW_S = harness.TEST_NOW_MS / 1000;
const HUB_ON: HubDeviceSupport = { state: "on" };
const MISSING = { kind: "missing_on_hub" } as const;
const ok = <T,>(data: T) => ({ kind: "ok" as const, data });
const restoreBackend = harness.installTestBackend();

function usageAnswer() {
	return {
		server_time: NOW_S,
		limits: {
			max_devices: 100,
			max_pending_enrollments: 10,
			enrollment_ttl_seconds: 86_400,
			max_enrollments_per_day: 220,
			max_account_backups: 256,
		},
		usage: {
			active_devices: 1,
			revoked_devices: 0,
			pending_enrollments: 1,
			enrollments_last_24h: 1,
			account_backups: 1,
		},
	};
}

/** What a current hub answers on the routes an older hub lacks. */
function currentHubAnswers() {
	return {
		ENROLLMENT: {
			enrollment_id: "enrollment-1",
			device_id: "dev-new",
			name: "factory-line-3",
			state: "pending" as const,
			created_at: NOW_S - 600,
			expires_at: NOW_S + 86_400,
			controller_key_thumbprint: "thumb",
		},
		BACKUPS: {
			vaults: [
				{
					key_id: "dev-1",
					revision: 3,
					updated_at: NOW_S - 9_000,
					public_key_thumbprint: "thumb",
				},
			],
			used: 1,
			max: 256,
		},
		FLEET_CERTIFICATES: [
			{
				device_id: "dev-1",
				revision: 2,
				updated_at: NOW_S - 60,
				certificates: [],
			},
		],
		SUMMARY: { server_time: NOW_S, devices: [] },
		ARCHIVE: {
			tier: "team",
			max_bytes: 1_000_000,
			retention_seconds: 86_400,
			used_bytes: 10,
			devices: [],
		},
	};
}

const USAGE = usageAnswer();
const { ENROLLMENT, BACKUPS, FLEET_CERTIFICATES, SUMMARY, ARCHIVE } =
	currentHubAnswers();

afterEach(dom.cleanup);
afterAll(() => {
	restoreBackend();
	dom.restore();
});

describe("buildAttentionInput", () => {
	test("older hub: every new route missing leaves its fields absent and keeps the planes it can read", async () => {
		const test = createTestWorkspace();
		await test.workspace.local.reload();
		const input = hooks.buildAttentionInput(test.workspace, {
			hub: HUB_ON,
			rows: [testRow("dev-1")],
			usage: MISSING,
			enrollments: MISSING,
			accountBackups: MISSING,
			certInventoryAll: MISSING,
			resourceSummary: MISSING,
			archiveUsage: MISSING,
		});
		expect(input.me).toBe(TEST_ME);
		expect(input.now).toBe(NOW_S);
		expect(input.devices.map((row) => row.device_id)).toEqual(["dev-1"]);
		expect(input.devicesLoaded).toBe(true);
		expect(input.usage).toBeUndefined();
		expect(input.accountBackupSlots).toBeUndefined();
		expect(input.accountBackups).toEqual({});
		expect(input.resourceSummary).toBeUndefined();
		expect(input.archiveUsage).toBeUndefined();
		expect(input.certInventory).toEqual({});
		expect(input.latestRelease).toBeUndefined();
		// BG2 interim: without the hub list only this computer's records count.
		expect(input.pendingSetups).toEqual([]);
		expect(input.local.vaults.map((vault) => vault.deviceId)).toEqual([
			"dev-1",
		]);
		expect(input.keys.map((row) => [row.deviceId, row.state])).toEqual([
			["dev-1", "locked"],
		]);
		expect(input.live).toEqual({});
		expect(input.fleet).toEqual({});
		expect(input.accessRequests).toEqual([]);
	});

	test("nothing loaded yet: no device list, no access-request verdicts, hub facts absent", () => {
		const test = createTestWorkspace();
		const input = hooks.buildAttentionInput(test.workspace, {
			hub: { state: "checking" },
		});
		expect(input.devices).toEqual([]);
		expect(input.devicesLoaded).toBe(false);
		expect(input.pendingSetups).toBeUndefined();
		expect(input.accessRequests).toBeUndefined();
	});

	test("current hub: usage, setups, backups, certificates, approvals, history and release are folded in", async () => {
		const test = createTestWorkspace();
		await test.workspace.local.reload();
		const input = hooks.buildAttentionInput(test.workspace, {
			hub: { ...HUB_ON, releaseTrust: { manifest_url: "https://r.test" } },
			rows: [testRow("dev-1")],
			readiness: { version: 1, ready: true, checks: [] },
			usage: ok(USAGE),
			enrollments: ok([ENROLLMENT]),
			accountBackups: ok(BACKUPS),
			certInventoryAll: ok(FLEET_CERTIFICATES),
			resourceSummary: ok(SUMMARY),
			archiveUsage: ok(ARCHIVE),
			release: {
				manifest: { release_version: "0.9.4", sequence: 44 },
			} as never,
		});
		expect(input.usage).toEqual({ limits: USAGE.limits, usage: USAGE.usage });
		expect(input.pendingSetups).toEqual([
			{
				enrollmentId: "enrollment-1",
				deviceId: "dev-new",
				name: "factory-line-3",
				state: "pending",
				createdAt: NOW_S - 600,
				expiresAt: NOW_S + 86_400,
				local: false,
			},
		]);
		expect(input.accountBackups).toEqual({
			"dev-1": { revision: 3, updatedAt: NOW_S - 9_000 },
		});
		expect(input.accountBackupSlots).toEqual({ used: 1, max: 256 });
		expect(input.certInventory["dev-1"]?.revision).toBe(2);
		expect(input.resourceSummary).toEqual(SUMMARY);
		expect(input.archiveUsage).toEqual(ARCHIVE);
		expect(input.readiness?.ready).toBe(true);
		expect(input.releaseTrust).toEqual({ manifest_url: "https://r.test" });
		expect(input.latestRelease).toEqual({ version: "0.9.4", sequence: 44 });
	});

	test("live facts: session, services, what views reported and per-device hub reads from the cache", async () => {
		const test = createTestWorkspace();
		const { workspace, queryClient } = test;
		await workspace.local.reload();
		await test.unlock("dev-1");
		workspace.facts.record("dev-1", {
			placements: { "svc-1": { host: "0.0.0.0", port: 8443 } },
		});
		queryClient.setQueryData(
			deviceKeys.myAccess(workspace.scopeKey, "dev-1"),
			ok({ device_id: "dev-1", grants: [] }),
		);
		queryClient.setQueryData(deviceKeys.policy(workspace.scopeKey, "dev-1"), {
			policy_jws: "jws",
			version: 4,
			digest: "d",
			applied_version: 3,
			applied_digest: "c",
		});
		const verified: string[] = [];
		const input = hooks.buildAttentionInput(workspace, {
			hub: HUB_ON,
			rows: [testRow("dev-1")],
			verifyPolicy: (deviceId, view) => {
				verified.push(`${deviceId}@${view.version}`);
				return { policy_version: view.version, grants: [] } as never;
			},
		});
		const live = input.live["dev-1"];
		expect(live?.state.kind).toBe("live");
		expect(live?.inspection?.value.placements.map((row) => row.id)).toEqual([
			"svc-1",
		]);
		expect(live?.placements).toEqual({
			"svc-1": { host: "0.0.0.0", port: 8443 },
		});
		expect(input.keys[0]?.state).toBe("unlocked");
		expect(input.myAccess?.["dev-1"]).toEqual({
			device_id: "dev-1",
			grants: [],
		} as never);
		expect(verified).toEqual(["dev-1@4"]);
		expect(input.policies["dev-1"]).toMatchObject({
			version: 4,
			applied_version: 3,
			policy: { policy_version: 4 },
		});
	});
});

describe("useAttention", () => {
	const mount = async (tickMs: number | false = false) => {
		const test = createTestWorkspace({
			routes: { "GET devices/controller-vaults": { ...BACKUPS, vaults: [] } },
		});
		const attention: Partial<Harness.Captured<Hooks.AttentionState>> = {};
		const clock: Partial<Harness.Captured<number>> = {};
		await dom.render(
			<TestProviders test={test} tickMs={tickMs}>
				<Capture read={hooks.useAttentionState} into={attention} />
				<Capture read={useAreaNow} into={clock} />
			</TestProviders>,
		);
		await test.idle();
		const seen = attention as Harness.Captured<Hooks.AttentionState>;
		const ticked = clock as Harness.Captured<number>;
		return { test, attention: seen, clock: ticked };
	};

	test("recomputes when a hub query updates, and never on the area's clock tick", async () => {
		const { test, attention, clock } = await mount(15);
		const { input } = attention.current;
		const renders = attention.renders;
		const ticks = clock.renders;

		for (let second = 0; second < 4; second++) {
			test.clock.nowMs += 1000;
			await advance(40);
		}
		expect(clock.renders - ticks).toBeGreaterThan(2);
		expect(clock.current).toBe(test.clock.nowMs);
		expect(attention.renders).toBe(renders);
		expect(attention.current.input).toBe(input);

		test.queryClient.setQueryData(deviceKeys.list(test.workspace.scopeKey), [
			testRow("dev-1"),
			testRow("dev-2"),
		]);
		await test.idle();
		expect(attention.current.input).not.toBe(input);
		expect(attention.current.input.devices.length).toBe(2);
	});

	test("recomputes when a manager changes (keys unlocked)", async () => {
		const { test, attention } = await mount();
		const before = attention.current.input;
		expect(before.keys.map((row) => row.state)).toEqual(["locked"]);
		await test.unlock("dev-1", false);
		await test.idle();
		const after = attention.current.input;
		expect(after.keys.map((row) => row.state)).toEqual(["unlocked"]);
		expect(after).not.toBe(before);
	});

	test("items carry the gate of their action, evaluated by the binding and never by the rules", async () => {
		const { test, attention } = await mount();
		const state = attention.current as Hooks.AttentionState;
		const backup = state.items.find(
			(item) => item.key === "keys_not_backed_up_to_account",
		);
		expect(backup?.action?.code).toBe("back_up_to_account");
		expect(backup?.action?.gate).toEqual(
			evaluateGate(
				"account_backup_save",
				hooks.buildGateContext(state, "dev-1"),
			),
		);
		expect(test.workspace.keys.snapshot("dev-1").state).toBe("locked");
		for (const item of state.items)
			if (item.action?.code === "diagnose")
				expect(item.action.gate).toBeUndefined();
	});

	test("filtering by device and counting read the one computation", async () => {
		const test = createTestWorkspace({
			rows: [
				testRow("dev-1"),
				testRow("dev-2", { last_seen_at: NOW_S - 86_400 }),
			],
			vaults: ["dev-1"],
			routes: { "GET devices/controller-vaults": { ...BACKUPS, vaults: [] } },
		});
		const all: Partial<Harness.Captured<unknown[]>> = {};
		const one: Partial<Harness.Captured<unknown[]>> = {};
		const counts: Partial<Harness.Captured<Hooks.AttentionCounts>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={() => hooks.useAttention()} into={all} />
				<Capture
					read={() => hooks.useAttention({ deviceId: "dev-2" })}
					into={one}
				/>
				<Capture read={() => hooks.useAttentionCounts()} into={counts} />
			</TestProviders>,
		);
		await test.idle();
		const ids = (items: unknown[] | undefined) =>
			(items as { id: string }[] | undefined)?.map((item) => item.id) ?? [];
		expect(ids(one.current).length).toBeGreaterThan(0);
		expect(ids(one.current).every((id) => id.includes("dev-2"))).toBe(true);
		expect(ids(all.current).length).toBeGreaterThan(ids(one.current).length);
		const {
			critical = 0,
			warning = 0,
			notice = 0,
			total = 0,
		} = counts.current ?? {};
		expect(total).toBe(critical + warning + notice);
	});

	test("status bar: six segments, the hub one current, encrypted status locked until a device is unlocked", async () => {
		const test = createTestWorkspace();
		const planes: Partial<Harness.Captured<Hooks.PlaneStatus>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={hooks.usePlaneStatus} into={planes} />
			</TestProviders>,
		);
		await test.idle();
		const byId = Object.fromEntries(
			(planes.current?.segments ?? []).map((segment) => [segment.id, segment]),
		);
		expect(Object.keys(byId)).toEqual([
			"hub",
			"status",
			"live",
			"local",
			"device",
			"certificates",
		]);
		expect(byId.hub).toMatchObject({ plane: "hub", count: 1, total: 1 });
		expect(byId.hub?.freshness.age).toBe("current");
		expect(byId.status?.freshness.age).toBe("locked");
		expect(byId.live).toMatchObject({ count: 0, total: 1 });
		expect(byId.local?.count).toBe(1);

		await test.unlock("dev-1");
		await test.idle();
		const live = planes.current?.segments.find((row) => row.id === "live");
		expect(live).toMatchObject({ count: 1, failing: 0 });
		expect(live?.freshness.age).toBe("live");
	});
});

describe("gates", () => {
	test("useGate follows the key session: locked, then allowed once unlocked and connected", async () => {
		const test = createTestWorkspace();
		const gate: Partial<Harness.Captured<unknown>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture
					read={() => hooks.useGate("stop", "dev-1", { placementId: "svc-1" })}
					into={gate}
				/>
			</TestProviders>,
		);
		await test.idle();
		expect(gate.current).toMatchObject({ ok: false, gate: "G7" });
		await test.unlock("dev-1");
		await test.idle();
		expect(gate.current).toEqual({ ok: true });
	});

	test("fix actions: unlock opens the overlay, places come back as a route", async () => {
		const test = createTestWorkspace();
		const fix: Partial<
			Harness.Captured<ReturnType<typeof hooks.useFixAction>>
		> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={hooks.useFixAction} into={fix} />
			</TestProviders>,
		);
		await test.idle();
		const run = fix.current as ReturnType<typeof hooks.useFixAction>;
		expect(
			run({ kind: "unlock", deviceId: "dev-1", connectLive: true }),
		).toEqual({ kind: "done" });
		expect(hooks.useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: "dev-1",
			connectLive: true,
		});
		hooks.useOverlayStore.getState().close();
		expect(run({ kind: "open_hub_status" })).toEqual({
			kind: "navigate",
			route: { screen: "hub" },
		});
		expect(run({ kind: "restore_keys", deviceId: "dev-1" })).toEqual({
			kind: "navigate",
			route: { screen: "keys", focusDeviceId: "dev-1" },
		});
		expect(run({ kind: "use_desktop" })).toEqual({
			kind: "external",
			what: "use_desktop",
		});
	});
});
