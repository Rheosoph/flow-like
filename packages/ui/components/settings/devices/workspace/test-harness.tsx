import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { ApiResponseError } from "../../../../lib/api-error";
import { base64url } from "../../../../lib/device-management/crypto";
import type {
	DeviceRow,
	InspectionPlus,
	PlacementStatusPlus,
} from "../../../../lib/device-management/model/types";
import type { LocalDeviceVault } from "../../../../lib/device-management/storage";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	ManagementResponse,
} from "../../../../lib/device-management/types";
import {
	type LiveConnectInput,
	type LiveConnection,
	READ_COMMANDS,
} from "../../../../lib/device-management/workspace/live";
import {
	type DeviceWorkspaceOptions,
	type DeviceWorkspaceRuntime,
	createDeviceWorkspace,
} from "../../../../lib/device-management/workspace/registry";
import type { WorkspaceDeps } from "../../../../lib/device-management/workspace/types";
import { useBackendStore } from "../../../../state/backend-state";
import type { IApiState } from "../../../../state/backend-state/api-state";
import type { IProfile } from "../../../../types";
import { ConfirmProvider } from "../primitives/confirm-sheet";
import { AreaProvider } from "./area-context";
import {
	type DeviceAuth,
	DeviceWorkspaceProvider,
} from "./device-workspace-provider";

/*
 * The inline minimal fake of this lane's tests: the real managers over an
 * in-memory hub, device and key store. No module mocks.
 */

export const TEST_PASSWORD = "correct horse";
export const TEST_ME = "usr_test_owner";
export const TEST_NOW_MS = 1_790_769_600_000;
export const TEST_PROFILE = {
	id: "profile-1",
	name: "Test",
	hub: "hub.test",
	bits: [],
	created: "",
	updated: "",
} as IProfile;
export const TEST_AUTH: DeviceAuth = {
	loading: false,
	signedIn: true,
	issuer: "https://issuer.test",
	account: TEST_ME,
};
export const TEST_SCOPE = {
	issuer: TEST_AUTH.issuer,
	account: TEST_ME,
	apiOrigin: "https://hub.test",
	profileId: "profile-1",
};

const key = (fill: number) => ({
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x: base64url(new Uint8Array(32).fill(fill)),
});

export function testIdentity(seed = 20): DeviceReceipt["identity"] {
	return {
		auth_key: key(seed),
		telemetry_key: key(seed + 1),
		management_key: Array(32).fill(seed + 2),
	};
}

export function testRow(
	deviceId: string,
	patch: Partial<DeviceRow> = {},
): DeviceRow {
	return {
		device_id: deviceId,
		owner_id: TEST_ME,
		name: deviceId,
		status: "active",
		registered_at: TEST_NOW_MS / 1000 - 86_400,
		last_seen_at: TEST_NOW_MS / 1000 - 20,
		auth_epoch: 1,
		identity: testIdentity(),
		...patch,
	} as DeviceRow;
}

export function testVault(deviceId: string, owner = true): LocalDeviceVault {
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
		controllerVault: new TextEncoder().encode(TEST_PASSWORD),
		invitationVault: owner ? new Uint8Array(80).fill(4) : undefined,
	};
}

export function testPlacement(
	id: string,
	patch: Partial<PlacementStatusPlus> = {},
): PlacementStatusPlus {
	return {
		id,
		project_id: "app-1",
		deployment_id: `${id}-deployment`,
		revision: `${id}-revision`,
		config_revision: 3,
		intent_revision: 3,
		applied_revision: 3,
		desired_state: "running",
		observed_state: "running",
		desired_replicas: 2,
		running_replicas: 2,
		ready_replicas: 2,
		max_replicas: 4,
		...patch,
	} as PlacementStatusPlus;
}

export type ApiCall = [method: string, path: string, body?: unknown];
type Route = unknown | ((body: unknown) => unknown);

export interface FakeHub {
	api: IApiState;
	calls: ApiCall[];
	/** `"GET devices"` → response or handler; anything else answers 404. */
	routes: Record<string, Route>;
	fetch: typeof fetch;
	hubJson: { status: number; body: unknown };
}

export function fakeHub(routes: Record<string, Route> = {}): FakeHub {
	const calls: ApiCall[] = [];
	const hub: FakeHub = {
		calls,
		routes,
		hubJson: { status: 200, body: { standalone: { enabled: true } } },
		api: undefined as never,
		fetch: undefined as never,
	};
	const answer = async (method: string, path: string, body?: unknown) => {
		calls.push(body === undefined ? [method, path] : [method, path, body]);
		const route = hub.routes[`${method} ${path}`];
		if (route === undefined)
			throw new ApiResponseError({
				status: 404,
				message: `No route ${method} ${path}`,
				path,
			});
		const value = typeof route === "function" ? route(body) : route;
		if (value instanceof Error) throw value;
		return structuredClone(value);
	};
	hub.api = {
		fetch: (_profile, path) => answer("GET", path) as never,
		get: (_profile, path) => answer("GET", path) as never,
		post: (_profile, path, data) => answer("POST", path, data) as never,
		put: (_profile, path, data) => answer("PUT", path, data) as never,
		patch: (_profile, path, data) => answer("PATCH", path, data) as never,
		del: (_profile, path, data) => answer("DELETE", path, data) as never,
		stream: async () => undefined,
	};
	hub.fetch = (async () =>
		new Response(JSON.stringify(hub.hubJson.body), {
			status: hub.hubJson.status,
			headers: { "content-type": "application/json" },
		})) as unknown as typeof fetch;
	return hub;
}

export interface FakeDevice {
	/** Every command that reached the device, in order (reads on connect included). */
	sent: Record<string, unknown>[];
	/** The commands that change something: what a test means by "sent". */
	readonly writes: Record<string, unknown>[];
	inspection: InspectionPlus;
	/** Answers a command; the default accepts it. */
	respond: (
		command: Record<string, unknown>,
	) => Partial<ManagementResponse> | Error | undefined;
	/** While set, the device answers no command until it resolves. */
	hold?: Promise<void>;
	connects: number;
}

function fakeDevice(
	deviceId: string,
	placements: PlacementStatusPlus[],
): FakeDevice {
	return {
		sent: [],
		get writes() {
			return this.sent.filter(
				(command) => !READ_COMMANDS.has(String(command.type)),
			);
		},
		connects: 0,
		respond: () => undefined,
		inspection: {
			device_id: deviceId,
			boot_id: "boot-1",
			placements,
			features: {},
		},
	};
}

export interface TestWorkspace {
	hub: FakeHub;
	deps: WorkspaceDeps;
	workspace: DeviceWorkspaceRuntime;
	queryClient: QueryClient;
	devices: Map<string, FakeDevice>;
	clock: { nowMs: number };
	/** Unlocks with the test password inside `act`; `connectLive` also opens the session and reads the services. */
	unlock(deviceId: string, connectLive?: boolean): Promise<void>;
	/** Flushes effects and waits until no hub query is fetching. */
	idle(): Promise<void>;
}

export interface TestWorkspaceOptions {
	rows?: DeviceRow[];
	/** Device ids with keys on this computer (owner vaults). */
	vaults?: string[];
	placements?: Record<string, PlacementStatusPlus[]>;
	routes?: Record<string, Route>;
	workspace?: DeviceWorkspaceOptions;
}

function fakeCrypto(vaults: Map<string, LocalDeviceVault>): DeviceCrypto {
	return {
		unlockControllerVault(
			deviceId: string,
			bytes: Uint8Array,
			ciphertext: Uint8Array,
		) {
			const decoder = new TextDecoder();
			if (decoder.decode(bytes) !== decoder.decode(ciphertext))
				throw new Error("aead::Error");
			const stored = vaults.get(deviceId) as LocalDeviceVault;
			return {
				publicBundle: () => stored.controllerPublic,
				close: () => undefined,
				free: () => undefined,
			} as unknown as BrowserController;
		},
		verifyDeviceReceipt(value: DeviceReceipt) {
			const stored = vaults.get(value.device_id) as LocalDeviceVault;
			return {
				device_id: value.device_id,
				api_base_url: `${TEST_SCOPE.apiOrigin}/api/v1`,
				owner_id: TEST_ME,
				controller_key:
					stored.ownerControllerKey ?? stored.controllerPublic.controller_key,
			};
		},
	} as unknown as DeviceCrypto;
}

function fakeConnection(
	device: FakeDevice,
	nowS: () => number,
): LiveConnection {
	let open = true;
	let sequence = 0;
	const closed = new Set<(reason: "local" | "remote") => void>();
	return {
		transport: "websocket",
		expiresAt: nowS() + 300,
		bootId: "boot-1",
		get open() {
			return open;
		},
		async request(command, operationId) {
			device.sent.push(command);
			const read = READ_COMMANDS.has(String(command.type));
			if (!read) await device.hold;
			const answer = read ? undefined : device.respond(command);
			if (answer instanceof Error) throw answer;
			sequence++;
			return {
				operation_id: operationId ?? `op-${sequence}`,
				state: "accepted",
				result: {},
				...answer,
			};
		},
		close() {
			if (!open) return;
			open = false;
			for (const listener of closed) listener("local");
		},
		onClosed(listener) {
			closed.add(listener);
			return () => closed.delete(listener);
		},
	};
}

/** Real managers over an in-memory hub and device; timers for polling are never started. */
export function createTestWorkspace(
	options: TestWorkspaceOptions = {},
): TestWorkspace {
	// The activity tray persists per scope: every test starts with an empty one.
	globalThis.localStorage?.clear();
	const clock = { nowMs: TEST_NOW_MS };
	const rows = options.rows ?? [testRow("dev-1")];
	const vaults = new Map(
		(options.vaults ?? rows.map((row) => row.device_id)).map((deviceId) => [
			deviceId,
			testVault(deviceId),
		]),
	);
	const devices = new Map(
		rows.map((row) => [
			row.device_id,
			fakeDevice(
				row.device_id,
				options.placements?.[row.device_id] ?? [testPlacement("svc-1")],
			),
		]),
	);
	const hub = fakeHub({
		"GET devices": () => rows,
		"GET devices/setup": { version: 1, ready: true, checks: [] },
		...Object.fromEntries(
			rows.map((row) => [
				`GET devices/${row.device_id}/identity`,
				{
					enrollment_id: "enrollment",
					device_id: row.device_id,
					owner_id: row.owner_id,
					name: row.name,
					identity: row.identity,
					manifest_jws: "manifest",
					binding_jws: "binding",
					registered_at: 1,
					auth_epoch: 1,
				} satisfies DeviceReceipt,
			]),
		),
		...options.routes,
	});
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const deps: WorkspaceDeps = {
		api: hub.api,
		profile: TEST_PROFILE,
		scope: TEST_SCOPE,
		queryClient,
		crypto: async () => fakeCrypto(vaults),
		platform: "desktop",
		now: () => clock.nowMs,
	};
	const nowS = () => Math.floor(clock.nowMs / 1000);
	const workspace = createDeviceWorkspace(deps, {
		storage: null,
		fetch: hub.fetch,
		every: () => () => undefined,
		visible: () => true,
		local: {
			listDeviceVaults: async () => [...vaults.values()],
			readDeviceIdentityPins: async () => [],
			listAccountRecoveryStates: async () => ({}),
			readCertificateAuthorities: async () => [],
			readStoragePersistence: async () => "persisted",
			deleteDeviceVault: async (_scope, deviceId) => {
				vaults.delete(deviceId);
			},
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
				throw new ApiResponseError({ status: 404, message: "no reader" });
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
			connect: async (input: LiveConnectInput) => {
				const device = devices.get(input.deviceId) as FakeDevice;
				device.connects++;
				for (const step of [
					"getting_pass",
					"reaching_device",
					"securing",
				] as const)
					input.onStep({ step, state: "done" });
				return fakeConnection(device, nowS);
			},
			readInspection: async (_call, deviceId) => ({
				...(devices.get(deviceId) as FakeDevice).inspection,
				observed_at: clock.nowMs,
			}),
			inventoryWriter: async () => async () => undefined,
			schedule: () => () => undefined,
		},
		streams: { schedule: () => () => undefined },
		...options.workspace,
	});
	return {
		hub,
		deps,
		workspace,
		queryClient,
		devices,
		clock,
		unlock: (deviceId, connectLive = true) =>
			act(async () => {
				await workspace.keys.unlock(deviceId, TEST_PASSWORD, { connectLive });
				if (connectLive) await workspace.live.refreshInspection(deviceId);
			}),
		async idle() {
			for (let round = 0; round < 25; round++) {
				await act(async () => {
					await new Promise((resolve) => setTimeout(resolve, 0));
				});
				if (!queryClient.isFetching()) break;
			}
			await act(async () => {
				await new Promise((resolve) => setTimeout(resolve, 0));
			});
		},
	};
}

/** `useDeveloperMode` and the profile lookup read the backend store: give them an inert one. */
export function installTestBackend(
	api: IApiState = fakeHub().api,
	extra: Record<string, unknown> = {},
): () => void {
	const previous = useBackendStore.getState().backend;
	useBackendStore.setState({
		backend: {
			userState: {
				getInfo: async () => ({ dev_mode: false }),
				getProfile: async () => TEST_PROFILE,
				updateUser: async () => undefined,
			},
			appState: {},
			eventState: {},
			apiState: api,
			...extra,
		} as never,
	});
	return () => useBackendStore.setState({ backend: previous });
}

export interface Captured<T> {
	current: T;
	renders: number;
}

/** Reads a hook inside the providers and hands its latest value to the test. */
export function Capture<T>({
	read,
	into,
}: Readonly<{ read: () => T; into: Partial<Captured<T>> }>) {
	into.current = read();
	into.renders = (into.renders ?? 0) + 1;
	return null;
}

/** The providers a screen runs under, bound to a test workspace. */
export function TestProviders({
	test,
	passive,
	fallback,
	auth = TEST_AUTH,
	tickMs = false,
	children,
}: Readonly<{
	test: TestWorkspace;
	passive?: boolean;
	fallback?: ReactNode;
	auth?: DeviceAuth;
	/** The area clock stands still unless a test asks for ticks. */
	tickMs?: number | false;
	children: ReactNode;
}>) {
	return (
		<QueryClientProvider client={test.queryClient}>
			<DeviceWorkspaceProvider
				passive={passive}
				fallback={fallback}
				overrides={{
					auth,
					profile: TEST_PROFILE,
					platform: "desktop",
					now: test.deps.now,
					workspace: () => test.workspace,
				}}
			>
				<AreaProvider tickMs={tickMs}>
					<ConfirmProvider>{children}</ConfirmProvider>
				</AreaProvider>
			</DeviceWorkspaceProvider>
		</QueryClientProvider>
	);
}
