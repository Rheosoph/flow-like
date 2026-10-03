import { afterEach, describe, expect, test } from "bun:test";
import { QueryClient } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { base64url } from "../crypto";
import { deviceKeys } from "../hub/queries";
import type { DeviceRow, InspectionPlus } from "../model/types";
import type { LocalDeviceVault } from "../storage";
import type { BrowserController, DeviceCrypto, DeviceReceipt } from "../types";
import type { LiveConnection } from "./live";
import {
	type DeviceWorkspaceOptions,
	type DeviceWorkspaceRuntime,
	createDeviceWorkspace,
	dismissWorkspaceSwitch,
	disposeAllDeviceWorkspaces,
	disposeDeviceWorkspace,
	getDeviceWorkspace,
	lastWorkspaceSwitch,
	openDeviceWorkspace,
	registerDeviceWorkspace,
	subscribeWorkspaceSwitch,
} from "./registry";
import type { WorkspaceDeps } from "./types";

const PASSWORD = "correct horse";
const NOW_MS = 1_790_769_600_000;
const ME = "owner";
const scope = {
	issuer: "issuer",
	account: ME,
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const profile = { id: "profile", hub: "hub.test" } as IProfile;
const flush = async () => {
	for (let round = 0; round < 8; round++)
		await new Promise((resolve) => setTimeout(resolve, 0));
};

const key = (fill: number) => ({
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x: base64url(new Uint8Array(32).fill(fill)),
});
const identity: DeviceReceipt["identity"] = {
	auth_key: key(20),
	telemetry_key: key(21),
	management_key: Array(32).fill(22),
};

function vault(deviceId: string, owner = true): LocalDeviceVault {
	return {
		deviceId,
		grantId: owner ? "owner" : "reader-grant",
		manifestJws: `manifest-${deviceId}`,
		ownerControllerKey: owner ? undefined : key(9),
		controllerPublic: {
			device_id: deviceId,
			endpoint_id: "endpoint",
			controller_key: key(1),
			archive_key: Array(32).fill(7),
			telemetry_member: { endpoint_id: "endpoint", signing_key: key(2) },
		},
		controllerVault: new TextEncoder().encode(PASSWORD),
		invitationVault: owner ? new Uint8Array(80).fill(4) : undefined,
	};
}

function row(deviceId: string, patch: Partial<DeviceRow> = {}): DeviceRow {
	return {
		device_id: deviceId,
		owner_id: ME,
		name: deviceId,
		status: "active",
		registered_at: NOW_MS / 1000 - 86_400,
		last_seen_at: NOW_MS / 1000 - 20,
		auth_epoch: 1,
		identity,
		...patch,
	} as DeviceRow;
}

interface Env {
	deps: WorkspaceDeps;
	options: DeviceWorkspaceOptions;
	client: QueryClient;
	calls: string[];
	vaults: Map<string, LocalDeviceVault>;
	rows: DeviceRow[];
	closed: { controllers: number; connections: number };
	connects: string[];
	storage: Map<string, string>;
	rechecks: (() => void)[];
	inspection: InspectionPlus;
	cryptoLoads: number;
}

function environment(
	setup: { vaults?: LocalDeviceVault[]; rows?: DeviceRow[] } = {},
): Env {
	const vaults = new Map(
		(setup.vaults ?? [vault("dev")]).map((value) => [value.deviceId, value]),
	);
	const env: Env = {
		deps: undefined as never,
		options: {},
		client: new QueryClient({ defaultOptions: { queries: { retry: false } } }),
		calls: [],
		vaults,
		rows: setup.rows ?? [row("dev")],
		closed: { controllers: 0, connections: 0 },
		connects: [],
		storage: new Map(),
		rechecks: [],
		cryptoLoads: 0,
		inspection: {
			device_id: "dev",
			boot_id: "boot-1",
			placements: [],
			features: {},
			agent: {
				version: "0.1.0",
				release_version: "0.9.4",
				release_sequence: 44,
			},
		},
	};
	const api = {
		get: async (_profile: IProfile, path: string) => {
			env.calls.push(`GET ${path}`);
			if (path === "devices") return structuredClone(env.rows);
			const match = /^devices\/([^/]+)\/identity$/u.exec(path);
			if (match)
				return {
					enrollment_id: "enrollment",
					device_id: match[1],
					owner_id: ME,
					name: match[1],
					identity,
					manifest_jws: "manifest",
					binding_jws: "binding",
					registered_at: 1,
					auth_epoch: 1,
				} satisfies DeviceReceipt;
			throw Object.assign(new Error(`No route ${path}`), { status: 404 });
		},
	} as unknown as IApiState;
	const crypto = {
		unlockControllerVault(deviceId: string, bytes: Uint8Array) {
			if (new TextDecoder().decode(bytes) !== PASSWORD)
				throw new Error("aead::Error");
			const stored = vaults.get(deviceId) as LocalDeviceVault;
			return {
				publicBundle: () => stored.controllerPublic,
				close: () => {
					env.closed.controllers++;
				},
				free: () => undefined,
			} as unknown as BrowserController;
		},
		verifyDeviceReceipt: (value: DeviceReceipt) => ({
			device_id: value.device_id,
			api_base_url: "https://hub.test/api/v1",
			owner_id: ME,
			controller_key: (vaults.get(value.device_id) as LocalDeviceVault)
				.controllerPublic.controller_key,
		}),
	} as unknown as DeviceCrypto;
	env.deps = {
		api,
		profile,
		scope,
		queryClient: env.client,
		crypto: async () => {
			env.cryptoLoads++;
			return crypto;
		},
		platform: "desktop",
		now: () => NOW_MS,
	};
	const connection = (): LiveConnection => {
		let open = true;
		return {
			transport: "websocket",
			expiresAt: NOW_MS / 1000 + 300,
			bootId: "boot-1",
			get open() {
				return open;
			},
			request: async (_command, operationId) => ({
				operation_id: operationId ?? "op",
				state: "accepted",
				result: {},
			}),
			close() {
				if (open) env.closed.connections++;
				open = false;
			},
			onClosed: () => () => undefined,
		};
	};
	env.options = {
		storage: {
			getItem: (name) => env.storage.get(name) ?? null,
			setItem: (name, value) => {
				env.storage.set(name, value);
			},
		},
		fetch: (async () =>
			new Response(
				JSON.stringify({ standalone: { enabled: true } }),
			)) as unknown as typeof fetch,
		every: (run) => {
			env.rechecks.push(run);
			return () => {
				env.rechecks.splice(env.rechecks.indexOf(run), 1);
			};
		},
		visible: () => true,
		local: {
			listDeviceVaults: async () => [...vaults.values()],
			readDeviceIdentityPins: async () => [],
			listAccountRecoveryStates: async () => ({}),
			readCertificateAuthorities: async () => [],
			readStoragePersistence: async () => "persisted",
		},
		keys: {
			readDeviceVault: async (_scope, deviceId) => vaults.get(deviceId),
			acquireDeviceLock: async () => () => undefined,
			queryDeviceLock: async () => "free",
			pinDeviceIdentity: async () => undefined,
			setTimer: () => 0,
			clearTimer: () => undefined,
			onPageHide: () => () => undefined,
		},
		fleet: {
			readFleet: async () => {
				throw Object.assign(new Error("no reader"), { status: 404 });
			},
			registerFleetReader: async () => undefined as never,
			removeFleetReader: async () => undefined as never,
			readSavedInventory: async () => [],
			setTimer: () => 0,
			clearTimer: () => undefined,
			visible: () => true,
			onVisibilityChange: () => () => undefined,
		},
		live: {
			connect: async (input) => {
				env.connects.push(input.deviceId);
				return connection();
			},
			readInspection: async () => ({ ...env.inspection, observed_at: NOW_MS }),
			inventoryWriter: async () => async () => undefined,
			schedule: () => () => undefined,
		},
		streams: { schedule: () => () => undefined },
	};
	return env;
}

const created: DeviceWorkspaceRuntime[] = [];
function workspaceOf(env: Env): DeviceWorkspaceRuntime {
	const workspace = createDeviceWorkspace(env.deps, env.options);
	created.push(workspace);
	return workspace;
}

afterEach(async () => {
	await disposeAllDeviceWorkspaces();
	for (const workspace of created.splice(0)) await workspace.dispose();
	dismissWorkspaceSwitch();
});

describe("composition", () => {
	test("wires every manager, the hub context and this computer's inventory", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		expect(workspace.scopeKey).toBe(JSON.stringify(Object.values(scope)));
		expect(workspace.hub.scopeKey).toBe(workspace.scopeKey);
		expect(workspace.local.summary().vaults.map((v) => v.deviceId)).toEqual([
			"dev",
		]);
		expect(workspace.keys.snapshot("dev").state).toBe("locked");
		expect(workspace.live.state("dev")).toEqual({ kind: "idle" });
		expect(workspace.activity.list()).toEqual([]);
		expect(workspace.attention.firstSeen).toEqual({});
	});

	test("late-bound ports: unlocking with connect live opens a session through the key manager", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		await workspace.keys.unlock("dev", PASSWORD, { connectLive: true });
		await workspace.live.refreshInspection("dev");
		expect(workspace.keys.snapshot("dev").state).toBe("unlocked");
		expect(env.connects).toEqual(["dev"]);
		expect(workspace.live.state("dev").kind).toBe("live");
		expect(workspace.live.inspection("dev")?.value.device_id).toBe("dev");
		expect(env.calls).toContain("GET devices/dev/identity");
	});

	test("the store version moves once per burst of manager changes", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		const before = workspace.store.getVersion();
		let notified = 0;
		workspace.store.subscribe(() => notified++);
		workspace.facts.record("dev", { rollouts: [] });
		workspace.facts.record("dev", { acme: [] });
		workspace.facts.record("dev", { metricReaders: [] });
		expect(workspace.store.getVersion()).toBe(before);
		await flush();
		expect(workspace.store.getVersion()).toBe(before + 1);
		expect(notified).toBe(1);
	});

	test("a hub query update of this scope moves the store version; another scope's does not", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		const before = workspace.store.getVersion();
		env.client.setQueryData(["devices", "other-scope", "list"], []);
		await flush();
		expect(workspace.store.getVersion()).toBe(before);
		env.client.setQueryData(deviceKeys.list(workspace.scopeKey), env.rows);
		await flush();
		expect(workspace.store.getVersion()).toBe(before + 1);
	});
});

describe("facts for the attention engine", () => {
	test("decrypted facts are merged per placement and dropped when the device locks", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		await workspace.keys.unlock("dev", PASSWORD);
		workspace.facts.record("dev", {
			placements: { a: { host: "0.0.0.0", port: 8443 } },
		});
		workspace.facts.record("dev", {
			placements: { b: { resourceGrantId: "grant" } },
			rollouts: [],
		});
		expect(workspace.facts.get("dev")).toEqual({
			placements: {
				a: { host: "0.0.0.0", port: 8443 },
				b: { resourceGrantId: "grant" },
			},
			rollouts: [],
		});
		workspace.keys.lock("dev");
		expect(workspace.facts.get("dev")).toBeUndefined();
	});

	test("the agent release of the last live read survives the lock and a new workspace", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		await workspace.keys.unlock("dev", PASSWORD, { connectLive: true });
		await workspace.live.refreshInspection("dev");
		const read = workspace.facts.agentLastRead().dev;
		expect(read).toMatchObject({ version: "0.9.4", sequence: 44 });
		workspace.keys.lock("dev");
		expect(workspace.facts.agentLastRead().dev).toEqual(read);
		await workspace.dispose();

		const again = workspaceOf(env);
		expect(again.facts.agentLastRead().dev).toEqual(read);
	});

	test("request keys get a record, turn approved once the hub lists the device, and go with the keys", async () => {
		const env = environment({
			vaults: [vault("dev"), vault("shared-pending", false)],
			rows: [row("dev")],
		});
		const workspace = workspaceOf(env);
		expect(workspace.facts.accessRequests()).toEqual([]);

		await flush();
		env.client.setQueryData(deviceKeys.list(workspace.scopeKey), env.rows);
		await flush();
		expect(workspace.facts.accessRequests()).toEqual([
			{
				deviceId: "shared-pending",
				createdAt: NOW_MS / 1000,
				approved: false,
			},
		]);

		workspace.facts.recordAccessRequest({
			deviceId: "shared-pending",
			deviceName: "lab-gpu-02",
			ownerId: "someone",
		});
		env.client.setQueryData(deviceKeys.list(workspace.scopeKey), [
			...env.rows,
			row("shared-pending", { owner_id: "someone" }),
		]);
		await flush();
		expect(workspace.facts.accessRequests()).toEqual([
			{
				deviceId: "shared-pending",
				deviceName: "lab-gpu-02",
				ownerId: "someone",
				createdAt: NOW_MS / 1000,
				approved: true,
			},
		]);

		env.vaults.delete("shared-pending");
		await workspace.local.reload();
		expect(workspace.facts.accessRequests()).toEqual([]);
	});
});

describe("registry lifecycle", () => {
	test("one workspace per scope: a second call returns it and rebinds api, profile and client only", async () => {
		const env = environment();
		const first = getDeviceWorkspace(env.deps, env.options);
		const nextApi = { ...env.deps.api } as IApiState;
		const nextProfile = { ...profile, name: "refreshed" } as IProfile;
		const second = getDeviceWorkspace({
			...env.deps,
			api: nextApi,
			profile: nextProfile,
			crypto: async () => {
				throw new Error("a later crypto loader must never replace the first");
			},
			now: () => 0,
			platform: "web",
		});
		expect(second).toBe(first);
		expect(first.deps.api).toBe(nextApi);
		expect(first.deps.profile).toBe(nextProfile);
		expect(first.hub.api).toBe(nextApi);
		expect(first.deps.platform).toBe("desktop");
		expect(first.deps.now?.()).toBe(NOW_MS);
		await first.deps.crypto();
		expect(env.cryptoLoads).toBeGreaterThan(0);
		expect(openDeviceWorkspace(first.scopeKey)).toBe(first);
	});

	test("another profile, hub or account locks every session, disposes the workspace and reports the switch", async () => {
		const env = environment();
		const first = getDeviceWorkspace(env.deps, env.options);
		await flush();
		await first.keys.unlock("dev", PASSWORD, { connectLive: true });
		await first.live.refreshInspection("dev");
		const seen: number[] = [];
		const stop = subscribeWorkspaceSwitch(() => seen.push(1));

		const other = environment();
		const second = getDeviceWorkspace(
			{
				...other.deps,
				scope: { ...scope, apiOrigin: "https://other.test", profileId: "p2" },
			},
			other.options,
		);
		await flush();
		stop();

		expect(second).not.toBe(first);
		expect(openDeviceWorkspace(first.scopeKey)).toBeUndefined();
		expect((first as DeviceWorkspaceRuntime).disposed).toBe(true);
		expect(first.keys.snapshot("dev").state).toBe("locked");
		expect(first.keys.controller("dev")).toBeUndefined();
		expect(first.live.state("dev")).toEqual({ kind: "idle" });
		expect(env.closed).toEqual({ controllers: 1, connections: 1 });
		expect(lastWorkspaceSwitch()).toMatchObject({
			from: scope,
			changed: ["hub", "profile"],
			lockedSessions: 1,
		});
		expect(seen.length).toBe(1);
		dismissWorkspaceSwitch();
		expect(lastWorkspaceSwitch()).toBeUndefined();
	});

	test("dispose by scope key is a sign-out: it reports the account, drops the scope's hub data and stops the timers", async () => {
		const env = environment();
		const workspace = getDeviceWorkspace(env.deps, env.options);
		await flush();
		env.client.setQueryData(deviceKeys.list(workspace.scopeKey), env.rows);
		expect(env.rechecks.length).toBe(1);

		await disposeDeviceWorkspace(workspace.scopeKey);

		expect(openDeviceWorkspace(workspace.scopeKey)).toBeUndefined();
		expect(lastWorkspaceSwitch()).toMatchObject({
			changed: ["account"],
			lockedSessions: 0,
		});
		expect(lastWorkspaceSwitch()?.to).toBeUndefined();
		expect(
			env.client.getQueryData(deviceKeys.list(workspace.scopeKey)),
		).toBeUndefined();
		expect(env.rechecks.length).toBe(0);
		const version = workspace.store.getVersion();
		workspace.facts.record("dev", { rollouts: [] });
		await flush();
		expect(workspace.store.getVersion()).toBe(version);
	});

	test("a workspace built over fakes can be registered and is the one the registry hands out", async () => {
		const env = environment();
		const fake = workspaceOf(env);
		registerDeviceWorkspace(fake);
		expect(getDeviceWorkspace(env.deps)).toBe(fake);
	});

	test("a workspace never moves to another account", () => {
		const env = environment();
		const workspace = workspaceOf(env);
		expect(() =>
			workspace.rebind({
				...env.deps,
				scope: { ...scope, account: "someone-else" },
			}),
		).toThrow("another account");
	});
});

describe("tray re-checks", () => {
	test("waiting items are re-read on the timer; nothing runs while nothing waits", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		const resumed: (string | undefined)[] = [];
		const resume = workspace.activity.resume.bind(workspace.activity);
		workspace.activity.resume = async (deviceId) => {
			resumed.push(deviceId);
			return resume(deviceId);
		};
		env.rechecks[0]?.();
		expect(resumed).toEqual([]);

		const id = workspace.activity.start({
			kind: "access_rules",
			target: { deviceId: "dev" },
			state: "active",
			label: { code: "access_rules" },
			startedBy: "you",
			actions: [],
			resume: { type: "policy", version: 3 },
		});
		workspace.activity.update(id, { state: "waiting" });
		env.rechecks[0]?.();
		expect(resumed).toEqual([undefined]);
	});

	test("keys that just opened re-read what the tray could not check while locked", async () => {
		const env = environment();
		const workspace = workspaceOf(env);
		await flush();
		const resumed: (string | undefined)[] = [];
		workspace.activity.resume = async (deviceId) => {
			resumed.push(deviceId);
		};
		await workspace.keys.unlock("dev", PASSWORD);
		expect(resumed).toEqual(["dev"]);
		workspace.keys.touch("dev");
		expect(resumed).toEqual(["dev"]);
	});
});
