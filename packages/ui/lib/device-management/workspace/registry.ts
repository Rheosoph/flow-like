import { type QueryClient, hashKey } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { toHubError } from "../hub/endpoints";
import {
	type HubQueryContext,
	deviceKeys,
	hubDeviceSupport,
	queries,
} from "../hub/queries";
import { createAttentionMemory } from "../model/attention";
import { convergence } from "../model/convergence";
import { presence, relationshipOf } from "../model/presence";
import type {
	AccessRequestRecord,
	AgentLastRead,
	DeviceRow,
	PlacementStatusPlus,
} from "../model/types";
import { type DeviceAccountScope, accountStorageKey } from "../storage";
import type { DeviceCrypto } from "../types";
import { createActivityTracker } from "./activity";
import { createClockModel } from "./clock";
import {
	type FleetReaderIo,
	type FleetReaderRuntime,
	createFleetSnapshotReader,
} from "./fleet-reader";
import {
	type KeyPreflightFacts,
	type KeySessionIo,
	type KeySessionRuntime,
	createKeySessionManager,
} from "./keys";
import {
	type LiveManagerOptions,
	type LiveSessionManagerImpl,
	createLiveSessionManager,
} from "./live";
import { type LocalInventoryIo, createLocalInventory } from "./local";
import { createWorkspaceStore } from "./store";
import {
	type LiveStreamsImpl,
	type StreamPorts,
	createLiveStreams,
} from "./streams";
import type {
	DeviceWorkspace,
	FleetPort,
	HubPort,
	KeyPort,
	KeySessionSnapshot,
	LiveFacts,
	LiveFactsStore,
	LiveInspection,
	LivePort,
	WorkspaceDeps,
} from "./types";

/*
 * M-DATA §3.1: one workspace per account scope, module-scoped so key sessions
 * and live connections survive route changes. Only this file wires concrete
 * managers (CA5); ports that point backwards are late-bound forwarders.
 */

type FactsStorage = Pick<Storage, "getItem" | "setItem">;

/** Seams for tests; production uses the browser and the lib defaults. */
export interface DeviceWorkspaceOptions {
	local?: Partial<LocalInventoryIo>;
	keys?: Partial<KeySessionIo>;
	fleet?: FleetReaderIo;
	live?: LiveManagerOptions;
	streams?: Omit<StreamPorts, "keys">;
	/** Attention memory and the facts kept on this computer; `null` keeps them in memory only. */
	storage?: FactsStorage | null;
	/** Fetch for the unauthenticated hub JSON. */
	fetch?: typeof fetch;
	/** Repeating timer for the tray's re-checks; returns its cancel. */
	every?: (run: () => void, ms: number) => () => void;
	visible?: () => boolean;
}

export interface DeviceWorkspaceRuntime extends DeviceWorkspace {
	readonly keys: KeySessionRuntime;
	readonly live: LiveSessionManagerImpl;
	readonly streams: LiveStreamsImpl;
	readonly fleet: FleetReaderRuntime;
	readonly disposed: boolean;
	/** A new backend object, query client or refreshed profile of the same scope; `crypto`, `now` and `platform` stay as created. */
	rebind(
		deps: Pick<WorkspaceDeps, "api" | "profile" | "queryClient" | "scope">,
	): void;
}

/** How often waiting tray items are re-read while the page is visible. */
export const ACTIVITY_RECHECK_MS = 20_000;
const AGENT_READ_WRITE_S = 300;
const FACTS_PREFIX = "flow-like.device-facts.";

function browserStorage(): FactsStorage | undefined {
	try {
		return globalThis.localStorage ?? undefined;
	} catch {
		return undefined;
	}
}

interface StoredFacts {
	agents: Record<string, AgentLastRead>;
	requests: AccessRequestRecord[];
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
	!!value && typeof value === "object" && !Array.isArray(value);

function storedAgents(value: unknown): StoredFacts["agents"] {
	if (!isRecord(value)) return {};
	const agents: StoredFacts["agents"] = {};
	for (const [deviceId, entry] of Object.entries(value)) {
		if (!isRecord(entry)) continue;
		const { version, sequence, at } = entry;
		if (typeof version !== "string" || typeof at !== "number") continue;
		agents[deviceId] = {
			version,
			sequence: typeof sequence === "number" ? sequence : null,
			at,
		};
	}
	return agents;
}

function storedRequests(value: unknown): AccessRequestRecord[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((entry) => {
		if (!isRecord(entry)) return [];
		const { deviceId, deviceName, ownerId, createdAt, approved } = entry;
		if (typeof deviceId !== "string" || typeof createdAt !== "number")
			return [];
		return [
			{
				deviceId,
				createdAt,
				approved: approved === true,
				...(typeof deviceName === "string" ? { deviceName } : {}),
				...(typeof ownerId === "string" ? { ownerId } : {}),
			},
		];
	});
}

function readStoredFacts(
	storage: FactsStorage | undefined,
	key: string,
): StoredFacts {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(key) ?? "{}");
		const value = isRecord(parsed) ? parsed : {};
		return {
			agents: storedAgents(value.agents),
			requests: storedRequests(value.requests),
		};
	} catch {
		return { agents: {}, requests: [] };
	}
}

interface FactsRuntime extends LiveFactsStore {
	noteInspection(deviceId: string, inspection: LiveInspection): void;
	reconcileAccessRequests(
		sharedVaultDeviceIds: readonly string[],
		listedDeviceIds: readonly string[] | undefined,
	): void;
	devices(): string[];
}

function createFactsStore(
	scopeKey: string,
	storage: FactsStorage | undefined,
	nowS: () => number,
): FactsRuntime {
	const key = `${FACTS_PREFIX}${scopeKey}`;
	const stored = readStoredFacts(storage, key);
	const live = new Map<string, LiveFacts>();
	const listeners = new Set<() => void>();
	let agents: Readonly<StoredFacts["agents"]> = stored.agents;
	let requests: readonly AccessRequestRecord[] = stored.requests;

	const emit = () => {
		for (const listener of [...listeners]) listener();
	};
	const save = () => {
		try {
			storage?.setItem(key, JSON.stringify({ agents, requests }));
		} catch {
			/* private window or quota: the facts stay in this tab */
		}
	};
	const setRequests = (next: readonly AccessRequestRecord[]) => {
		requests = next;
		save();
		emit();
	};

	return {
		get: (deviceId) => live.get(deviceId),
		record(deviceId, facts) {
			const current = live.get(deviceId) ?? {};
			const next: LiveFacts = { ...current, ...facts };
			if (facts.placements)
				next.placements = { ...current.placements, ...facts.placements };
			if (facts.offlineQueues)
				next.offlineQueues = {
					...current.offlineQueues,
					...facts.offlineQueues,
				};
			live.set(deviceId, next);
			emit();
		},
		clear(deviceId) {
			if (deviceId === undefined ? live.size === 0 : !live.has(deviceId))
				return;
			if (deviceId === undefined) live.clear();
			else live.delete(deviceId);
			emit();
		},
		devices: () => [...live.keys()],
		agentLastRead: () => agents,
		noteInspection(deviceId, inspection) {
			const release = inspection.value.agent;
			if (!release?.release_version) return;
			const known = agents[deviceId];
			const next: AgentLastRead = {
				version: release.release_version,
				sequence: release.release_sequence ?? null,
				at: inspection.readAt,
			};
			const same =
				known?.version === next.version && known.sequence === next.sequence;
			if (same && next.at - known.at < AGENT_READ_WRITE_S) return;
			agents = { ...agents, [deviceId]: next };
			save();
			emit();
		},
		accessRequests: () => requests,
		recordAccessRequest(request) {
			const known = requests.find((row) => row.deviceId === request.deviceId);
			const next: AccessRequestRecord = {
				...known,
				...request,
				createdAt: request.createdAt ?? known?.createdAt ?? nowS(),
				approved: request.approved ?? known?.approved ?? false,
			};
			setRequests([
				...requests.filter((row) => row.deviceId !== request.deviceId),
				next,
			]);
		},
		reconcileAccessRequests(sharedVaultDeviceIds, listedDeviceIds) {
			const shared = new Set(sharedVaultDeviceIds);
			const listed = new Set(listedDeviceIds ?? []);
			const kept = requests.filter((row) => shared.has(row.deviceId));
			let changed = kept.length !== requests.length;
			const next = kept.map((row) => {
				if (row.approved || !listed.has(row.deviceId)) return row;
				changed = true;
				return { ...row, approved: true };
			});
			const known = new Set(next.map((row) => row.deviceId));
			for (const deviceId of shared) {
				if (known.has(deviceId)) continue;
				changed = true;
				next.push({
					deviceId,
					createdAt: nowS(),
					approved: listed.has(deviceId),
				});
			}
			if (changed) setRequests(next);
		},
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
	};
}

const HUB_METHODS: Record<
	string,
	(
		api: IApiState,
		profile: IProfile,
		path: string,
		body: unknown,
	) => Promise<unknown>
> = {
	GET: (api, profile, path) => api.get(profile, path),
	POST: (api, profile, path, body) => api.post(profile, path, body),
	PUT: (api, profile, path, body) => api.put(profile, path, body),
	PATCH: (api, profile, path, body) => api.patch(profile, path, body),
	DELETE: (api, profile, path, body) => api.del(profile, path, body),
};

/** Newest row per placement id; the greyed "Locked · last read" list. */
function serviceSummaries(
	placements: readonly PlacementStatusPlus[],
): NonNullable<KeySessionSnapshot["lockedSummary"]>["services"] {
	const byId = new Map(placements.map((row) => [row.id, row]));
	return [...byId.values()].map((row) => ({
		serviceId: row.id,
		projectId: row.project_id,
		desired: row.desired_state,
		observed: row.observed_state,
		conv: convergence(row),
	}));
}

export function createDeviceWorkspace(
	deps: WorkspaceDeps,
	options: DeviceWorkspaceOptions = {},
): DeviceWorkspaceRuntime {
	const scopeKey = accountStorageKey(deps.scope);
	const { scope, platform } = deps;
	const localNow = deps.now ?? Date.now;
	const holder = {
		api: deps.api,
		profile: deps.profile,
		queryClient: deps.queryClient,
		crypto: deps.crypto,
	};
	const bound: WorkspaceDeps = {
		get api() {
			return holder.api;
		},
		get profile() {
			return holder.profile;
		},
		get queryClient() {
			return holder.queryClient;
		},
		crypto: (): Promise<DeviceCrypto> => holder.crypto(),
		scope,
		platform,
		now: localNow,
	};

	const store = createWorkspaceStore();
	let disposed = false;
	let bumpQueued = false;
	const bump = () => {
		if (bumpQueued || disposed) return;
		bumpQueued = true;
		queueMicrotask(() => {
			bumpQueued = false;
			if (!disposed) store.bump();
		});
	};

	const clock = createClockModel(bound);
	const nowS = () => Math.floor(clock.now() / 1000);
	const storage =
		options.storage === null
			? undefined
			: (options.storage ?? browserStorage());
	const facts = createFactsStore(scopeKey, storage, nowS);
	const attention = createAttentionMemory(scopeKey, storage);

	const hubContext: HubQueryContext = {
		get api() {
			return holder.api;
		},
		get profile() {
			return holder.profile;
		},
		scopeKey,
		clock,
		now: localNow,
		...(options.fetch ? { fetch: options.fetch } : {}),
	};
	const hub: HubPort = {
		fetch<T>(path: string, init?: { method?: string; body?: unknown }) {
			const method = (init?.method ?? "GET").toUpperCase();
			const send = HUB_METHODS[method];
			if (!send)
				return Promise.reject(
					new Error(`Unsupported hub request method ${method} for ${path}.`),
				);
			return send(holder.api, holder.profile, path, init?.body) as Promise<T>;
		},
	};

	const rows = () =>
		holder.queryClient.getQueryData<DeviceRow[]>(deviceKeys.list(scopeKey));
	const rowOf = (deviceId: string) =>
		rows()?.find((row) => row.device_id === deviceId);

	/** Filled in creation order below; the forwarding ports run only once all three exist. */
	const late = {} as {
		keys: KeySessionRuntime;
		live: LiveSessionManagerImpl;
		fleet: FleetReaderRuntime;
	};
	const keyPort: KeyPort = {
		controller: (deviceId) => late.keys.controller(deviceId),
		vault: (deviceId) => late.keys.vault(deviceId),
		receipt: (deviceId) => late.keys.receipt(deviceId),
		snapshot: (deviceId) => late.keys.snapshot(deviceId),
		subscribe: (listener) => late.keys.subscribe(listener),
		touch: (deviceId) => late.keys.touch(deviceId),
	};
	const livePort: LivePort = {
		acquire: (deviceId, reason) => late.live.acquire(deviceId, reason),
		call: (deviceId, callOptions) => late.live.call(deviceId, callOptions),
		exclusive: (deviceId, run, runOptions) =>
			late.live.exclusive(deviceId, run, runOptions),
		state: (deviceId) => late.live.state(deviceId),
		close: (deviceId) => late.live.close(deviceId),
	};
	const fleetPort: FleetPort = {
		watch: (deviceId) => late.fleet.watch(deviceId),
		refresh: (deviceId) => late.fleet.refresh(deviceId),
		get: (deviceId) => late.fleet.get(deviceId),
	};

	async function preflightFacts(deviceId: string): Promise<KeyPreflightFacts> {
		const client = holder.queryClient;
		const [hubRead, listRead] = await Promise.allSettled([
			client.ensureQueryData(queries.hub(hubContext)),
			client.ensureQueryData(queries.list(hubContext)),
		]);
		const device =
			listRead.status === "fulfilled"
				? listRead.value.find((row) => row.device_id === deviceId)
				: undefined;
		const restricted =
			listRead.status === "rejected" &&
			toHubError(listRead.reason).code === "token_restricted";
		return {
			now: nowS(),
			hub: hubDeviceSupport(
				hubRead.status === "fulfilled"
					? { data: hubRead.value }
					: { error: hubRead.reason },
				{ data: client.getQueryData(deviceKeys.usage(scopeKey)) },
			),
			auth: { signedIn: Boolean(scope.account), tokenScopeAll: !restricted },
			device,
			relationship: device ? relationshipOf(device, scope.account) : "unknown",
			clock: {
				hubOffsetS: clock.hubOffsetS,
				deviceSkewS: clock.deviceSkewS(deviceId),
			},
		};
	}

	function lockedSummary(
		deviceId: string,
	): KeySessionSnapshot["lockedSummary"] {
		const inspection = late.live.inspection(deviceId);
		const status = late.fleet.get(deviceId)?.status;
		if (inspection && (!status || inspection.readAt >= status.observedAt))
			return {
				readAt: inspection.readAt,
				services: serviceSummaries(inspection.value.placements),
			};
		if (!status) return undefined;
		return {
			readAt: status.observedAt,
			services: serviceSummaries(
				status.observations.flatMap((row) => row.placements),
			),
		};
	}

	const local = createLocalInventory(bound, options.local);
	const activity = createActivityTracker(bound, {
		live: livePort,
		keys: keyPort,
		hub,
	});
	const keys = createKeySessionManager(
		bound,
		{
			local,
			live: livePort,
			fleet: fleetPort,
			hub,
			activity,
			facts: preflightFacts,
			lockedSummary,
		},
		options.keys,
	);
	late.keys = keys;
	const fleet = createFleetSnapshotReader(
		bound,
		{
			keys,
			hub,
			clock,
			devices: () =>
				rows()
					?.filter((row) => row.status !== "revoked")
					.map((row) => row.device_id) ??
				keys.list().map((session) => session.deviceId),
		},
		options.fleet,
	);
	late.fleet = fleet;
	const live = createLiveSessionManager(
		bound,
		{
			keys: keyPort,
			hub,
			clock,
			activity,
			presence: (deviceId) => {
				const row = rowOf(deviceId);
				return row ? presence(row, nowS()).kind : "online";
			},
		},
		{
			...options.live,
			onInspection(deviceId, inspection) {
				facts.noteInspection(deviceId, inspection);
				options.live?.onInspection?.(deviceId, inspection);
			},
		},
	);
	late.live = live;
	const streams = createLiveStreams(bound, live, {
		...options.streams,
		keys: keyPort,
	});

	let localLoaded = false;
	const reconcileRequests = () => {
		if (!localLoaded || disposed) return;
		facts.reconcileAccessRequests(
			local
				.summary()
				.vaults.filter((vault) => vault.role === "shared")
				.map((vault) => vault.deviceId),
			rows()?.map((row) => row.device_id),
		);
	};

	/** Keys just opened: the tray re-reads what it could not check while locked. */
	const unlocked = new Set<string>();
	const onKeys = () => {
		const open = new Set(
			keys
				.list()
				.filter((session) => session.state === "unlocked")
				.map((session) => session.deviceId),
		);
		for (const deviceId of facts.devices())
			if (!open.has(deviceId)) facts.clear(deviceId);
		for (const deviceId of open)
			if (!unlocked.has(deviceId))
				void activity.resume(deviceId).catch(() => undefined);
		unlocked.clear();
		for (const deviceId of open) unlocked.add(deviceId);
		bump();
	};

	const listHash = hashKey(deviceKeys.list(scopeKey));
	const watchQueries = (client: QueryClient) =>
		client.getQueryCache().subscribe((event) => {
			if (event.type !== "updated" || event.action.type !== "success") return;
			const key = event.query.queryKey;
			if (key[0] !== "devices" || key[1] !== scopeKey) return;
			if (event.query.queryHash === listHash) {
				live.presenceChanged();
				reconcileRequests();
			}
			bump();
		});
	let stopQueries = watchQueries(holder.queryClient);

	const stops = [
		local.subscribe(() => {
			reconcileRequests();
			bump();
		}),
		keys.subscribe(onKeys),
		live.subscribe(bump),
		fleet.subscribe(bump),
		activity.subscribe(bump),
		facts.subscribe(bump),
	];

	const visible =
		options.visible ??
		(() => globalThis.document?.visibilityState !== "hidden");
	const every =
		options.every ??
		((run: () => void, ms: number) => {
			const timer = setInterval(run, ms);
			return () => clearInterval(timer);
		});
	const stopRechecks = every(() => {
		if (disposed || !visible()) return;
		const waiting = activity
			.list()
			.some((item) => item.resume && item.state === "waiting");
		if (waiting) void activity.resume().catch(() => undefined);
	}, ACTIVITY_RECHECK_MS);

	void local
		.reload()
		.catch(() => undefined)
		.then(() => {
			if (disposed) return;
			localLoaded = true;
			reconcileRequests();
			return activity.resume();
		})
		.catch(() => undefined);

	return {
		scopeKey,
		deps: bound,
		local,
		keys,
		live,
		streams,
		fleet,
		activity,
		clock,
		store,
		hub: hubContext,
		facts,
		attention,
		get disposed() {
			return disposed;
		},
		touch: (deviceId) => keys.touch(deviceId),
		rebind(next) {
			if (accountStorageKey(next.scope) !== scopeKey)
				throw new Error("A device workspace cannot move to another account.");
			holder.api = next.api;
			holder.profile = next.profile;
			if (holder.queryClient === next.queryClient || disposed) return;
			stopQueries();
			holder.queryClient = next.queryClient;
			stopQueries = watchQueries(next.queryClient);
		},
		async dispose() {
			if (disposed) return;
			disposed = true;
			stopRechecks();
			stopQueries();
			for (const stop of stops) stop();
			keys.dispose();
			live.dispose();
			streams.dispose();
			fleet.dispose();
			facts.clear();
		},
	};
}

/* Module registry: at most one scope is open; another account, hub or profile locks and disposes the rest. */

export type ScopeChange = "account" | "hub" | "profile";

export interface WorkspaceSwitch {
	from: DeviceAccountScope;
	/** Absent after a sign-out. */
	to?: DeviceAccountScope;
	changed: ScopeChange[];
	/** Key sessions that were unlocked and are locked now. */
	lockedSessions: number;
	/** Epoch milliseconds. */
	at: number;
}

const workspaces = new Map<string, DeviceWorkspaceRuntime>();
const switchListeners = new Set<() => void>();
let lastSwitch: WorkspaceSwitch | undefined;

function scopeChanges(
	from: DeviceAccountScope,
	to: DeviceAccountScope | undefined,
): ScopeChange[] {
	if (!to) return ["account"];
	const changed: ScopeChange[] = [];
	if (from.account !== to.account || from.issuer !== to.issuer)
		changed.push("account");
	if (from.apiOrigin !== to.apiOrigin) changed.push("hub");
	if (from.profileId !== to.profileId) changed.push("profile");
	return changed;
}

function unlockedCount(workspace: DeviceWorkspace): number {
	return workspace.keys.list().filter((session) => session.state === "unlocked")
		.length;
}

function setSwitch(next: WorkspaceSwitch | undefined) {
	lastSwitch = next;
	for (const listener of [...switchListeners]) listener();
}

async function closeWorkspace(
	scopeKey: string,
	to: DeviceAccountScope | undefined,
): Promise<void> {
	const workspace = workspaces.get(scopeKey);
	if (!workspace) return;
	workspaces.delete(scopeKey);
	const { scope, queryClient } = workspace.deps;
	setSwitch({
		from: scope,
		...(to ? { to } : {}),
		changed: scopeChanges(scope, to),
		lockedSessions: unlockedCount(workspace),
		at: Date.now(),
	});
	await workspace.dispose();
	queryClient.removeQueries({ queryKey: deviceKeys.root(scopeKey) });
}

/**
 * Makes an already built workspace the open one of its scope (tests build
 * theirs over fakes with `createDeviceWorkspace`); other scopes are closed.
 */
export function registerDeviceWorkspace(
	workspace: DeviceWorkspaceRuntime,
): void {
	for (const key of [...workspaces.keys()])
		if (key !== workspace.scopeKey)
			void closeWorkspace(key, workspace.deps.scope);
	workspaces.set(workspace.scopeKey, workspace);
}

/**
 * Creates the scope's workspace, or returns the open one rebound to the
 * current `api`, `profile` and `queryClient`. An open workspace keeps the
 * `crypto`, `now` and `platform` it was created with.
 */
export function getDeviceWorkspace(
	deps: WorkspaceDeps,
	options?: DeviceWorkspaceOptions,
): DeviceWorkspace {
	const scopeKey = accountStorageKey(deps.scope);
	for (const key of [...workspaces.keys()])
		if (key !== scopeKey) void closeWorkspace(key, deps.scope);
	const open = workspaces.get(scopeKey);
	if (open) {
		open.rebind(deps);
		return open;
	}
	const created = createDeviceWorkspace(deps, options);
	workspaces.set(scopeKey, created);
	return created;
}

/** Lock all, close all, stop timers (scope change, sign-out). */
export function disposeDeviceWorkspace(scopeKey: string): Promise<void> {
	return closeWorkspace(scopeKey, undefined);
}

export async function disposeAllDeviceWorkspaces(): Promise<void> {
	await Promise.all(
		[...workspaces.keys()].map((key) => closeWorkspace(key, undefined)),
	);
}

export function openDeviceWorkspace(
	scopeKey: string,
): DeviceWorkspace | undefined {
	return workspaces.get(scopeKey);
}

/** The latest scope change that disposed a workspace (S02 notice), until dismissed. */
export function lastWorkspaceSwitch(): WorkspaceSwitch | undefined {
	return lastSwitch;
}

export function dismissWorkspaceSwitch(): void {
	if (lastSwitch) setSwitch(undefined);
}

export function subscribeWorkspaceSwitch(listener: () => void): () => void {
	switchListeners.add(listener);
	return () => switchListeners.delete(listener);
}
