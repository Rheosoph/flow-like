import type { QueryClient } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import type { FleetMetrics } from "../fleet";
import type { HubQueryContext } from "../hub/queries";
import type { RetainedObservation } from "../inventory";
import type {
	AccessRequestRecord,
	AgentLastRead,
	AttentionMemory,
	CopyRef,
	DeviceIdentity,
	FixAction,
	Freshness,
	InspectionPlus,
	LiveDeviceInput,
	NavTarget,
	PlacementStatusPlus,
	PreflightCode,
	PreflightId,
	ServiceView,
	SourcePlane,
} from "../model/types";
import type { DeviceAccountScope, LocalDeviceVault } from "../storage";
import type { ManagementCall } from "../telemetry";
import type {
	ArchiveRoster,
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	ManagementGrant,
	ManagementPolicy,
	ManagementRejection,
	TelemetryRoster,
} from "../types";

/* Root (M-DATA §3.1, CA10). */

export type PlaneId = SourcePlane;

export interface WorkspaceDeps {
	api: IApiState;
	profile: IProfile;
	scope: DeviceAccountScope;
	queryClient: QueryClient;
	crypto: () => Promise<DeviceCrypto>;
	platform: "desktop" | "web";
	/** Milliseconds; injectable for tests. */
	now?: () => number;
}

export interface WorkspaceStore {
	getVersion(): number;
	bump(): void;
	subscribe(listener: () => void): () => void;
}

export interface DeviceWorkspace {
	/** `accountStorageKey(scope)`. */
	readonly scopeKey: string;
	readonly deps: WorkspaceDeps;
	readonly local: LocalInventory;
	readonly keys: KeySessionManager;
	readonly live: LiveSessionManager;
	readonly streams: LiveStreams;
	readonly fleet: FleetSnapshotReader;
	readonly activity: ActivityTracker;
	readonly clock: ClockModel;
	readonly store: WorkspaceStore;
	/** Hub query context of this scope: `queries.x(workspace.hub, …)` always reads the current api and profile. */
	readonly hub: HubQueryContext;
	/** What views read beyond the inspection, for the attention engine. */
	readonly facts: LiveFactsStore;
	/** Dwell and snooze memory of the attention engine, kept per scope on this computer. */
	readonly attention: AttentionMemory;
	/** Bump the "used" clock for a device (views, user actions). Background polling never calls it. */
	touch(deviceId: string): void;
	/** Lock all, close sessions, stop timers. */
	dispose(): Promise<void>;
}

/* Facts views report (CA11 `LiveDeviceInput` beyond state and inspection). */

export type LiveFacts = Omit<LiveDeviceInput, "state" | "inspection">;

export interface LiveFactsStore {
	/** Decrypted facts of one device; gone when its keys lock. */
	get(deviceId: string): LiveFacts | undefined;
	/** Replaces the given fields; `placements` and `offlineQueues` merge per placement id. */
	record(deviceId: string, facts: Partial<LiveFacts>): void;
	clear(deviceId?: string): void;
	/** Agent release per device from the last live read; survives locks and reloads (BG8 replaces it). */
	agentLastRead(): Readonly<Record<string, AgentLastRead | undefined>>;
	/**
	 * Access requests made from this computer; survives reloads (BG23 replaces it).
	 * Request keys without a record get one, a device the hub lists is approved,
	 * and a record whose keys are gone is dropped.
	 */
	accessRequests(): readonly AccessRequestRecord[];
	/** The access flow adds the device name and owner when it creates a request. */
	recordAccessRequest(
		request: Omit<AccessRequestRecord, "approved" | "createdAt"> &
			Partial<Pick<AccessRequestRecord, "approved" | "createdAt">>,
	): void;
	subscribe(listener: () => void): () => void;
}

/* Key sessions (M-DATA §3.2). */

export type KeyState =
	| "none"
	| "stale"
	| "locked"
	| "unlocking"
	| "unlocked"
	| "held_elsewhere"
	| "blocked";

export type ServiceSummary = Pick<
	ServiceView,
	"serviceId" | "projectId" | "desired" | "observed" | "conv"
>;

export interface KeySessionSnapshot {
	deviceId: string;
	state: KeyState;
	role: "owner" | "shared";
	grantId: string;
	/** Owner with the invitation handle attached (§3.2.4). */
	canSign: boolean;
	/** `unlockedAt`, `lastUsedAt` and `idleLocksAt` are epoch milliseconds of `WorkspaceDeps.now`. */
	unlockedAt?: number;
	lastUsedAt?: number;
	/** Undefined while "keep unlocked" is on. */
	idleLocksAt?: number;
	keepUnlocked: boolean;
	restoredNeedsFreshEndpoint: boolean;
	/** Greyed "Locked · last read 12:03"; `readAt` is unix seconds. */
	lockedSummary?: { readAt: number; services: ServiceSummary[] };
	lastError?: KeyError;
}

export type KeyError =
	| { code: "wrong_password" }
	| { code: "no_vault" }
	| { code: "held_elsewhere" }
	| { code: "lock_unsupported" }
	| { code: "crypto_unavailable" }
	| {
			code: "identity_mismatch";
			pinnedAt: number;
			fingerprint: string;
			reported: string;
	  }
	| { code: "authority_mismatch" }
	| { code: "storage"; detail: string };

export interface UnlockOptions {
	connectLive?: boolean;
	/** Seal and upload the account backup with the typed password (P2). */
	backupToAccount?: boolean;
	keepUnlocked?: boolean;
	signal?: AbortSignal;
	onProgress?: (step: UnlockStep) => void;
}

export type UnlockStepId =
	| "unlocking_keys"
	| "checking_identity"
	| "rotating_endpoint"
	| "saving_backup"
	| "reading_encrypted_status"
	| "getting_pass"
	| "reaching_device"
	| "trying_direct"
	| "securing"
	| "reading_services";

export type StepDetailCode =
	| "relay_no_turn_servers"
	| "relay_ice_timeout"
	| "relay_webrtc_failed"
	| "relay_webrtc_unavailable"
	| "fresh_endpoint"
	| "backup_saved"
	| "backup_local_only"
	| "backup_limit"
	| "slots_in_use"
	| "not_requested"
	| "cancelled"
	| "wrong_password"
	| "no_vault"
	| "held_elsewhere"
	| "lock_unsupported"
	| "crypto_unavailable"
	| "identity_mismatch"
	| "authority_mismatch"
	| "storage_error"
	| "not_configured"
	| "needs_wss"
	| "access_expired"
	| "epoch_mismatch"
	| "invalid_admission"
	| "http_error"
	| "relay_unreachable"
	| "handshake_failed"
	| "identity_confirmation_failed"
	| "rejected"
	| "session_closed"
	| "timeout"
	| "expired";

export interface UnlockStep {
	id: UnlockStepId;
	state: "pending" | "active" | "done" | "skipped" | "failed";
	detail?: CopyRef<StepDetailCode>;
}

export interface PreflightRow {
	/** Shown only in Copy diagnostics (R3). D10–D13 are evaluated during and after connecting. */
	id: PreflightId;
	status: "pass" | "warn" | "fail" | "block" | "checking";
	source: PlaneId;
	copy: CopyRef<PreflightCode>;
	fix?: FixAction;
	/** Further fixes after the primary one (D4: Restore · Import file · Request access). */
	otherFixes?: FixAction[];
}

export interface Preflight {
	rows: PreflightRow[];
	/** False on any "block" or "fail" in D1–D7. */
	passwordEnabled: boolean;
	/** False when D8 says offline. */
	suggestConnectLive: boolean;
	/** Plain-text report for support, no secrets. */
	diagnostics(): string;
}

export type UnlockManyOutcome =
	| "unlocked"
	| "wrong_password"
	| "no_vault"
	| "held_elsewhere"
	| "error";

export interface VaultLease {
	vault: LocalDeviceVault;
	/** After rewrap, restore or endpoint rotation. */
	replace(next: LocalDeviceVault): void;
}

export interface OwnerSigner {
	signPolicy(policy: ManagementPolicy, password?: string): Promise<string>;
	signTelemetryRoster(
		roster: TelemetryRoster,
		password?: string,
	): Promise<string>;
	signArchiveRoster(roster: ArchiveRoster, password?: string): Promise<string>;
}

export interface KeySessionManager {
	snapshot(deviceId: string): KeySessionSnapshot;
	/** Every device with a vault, plus open sessions. */
	list(): KeySessionSnapshot[];
	subscribe(listener: () => void): () => void;
	/** D1–D9; needs no keys. */
	preflight(deviceId: string): Promise<Preflight>;
	unlock(
		deviceId: string,
		password: string,
		options?: UnlockOptions,
	): Promise<void>;
	/** Sequential (IA §6.4.4). */
	unlockMany(
		deviceIds: string[],
		password: string,
		onEach: (deviceId: string, outcome: UnlockManyOutcome) => void,
		signal?: AbortSignal,
	): Promise<void>;
	/** Web Locks steal ("Use here"). */
	takeOver(deviceId: string): Promise<void>;
	/** Clears keys, live data and decrypted history; keeps `lockedSummary`. */
	lock(deviceId: string): void;
	lockAll(): void;
	setKeepUnlocked(deviceId: string, keep: boolean): void;
	/** Run a vault-mutating local operation under this device's lock without a second lock. */
	withVaultLease<T>(
		deviceId: string,
		run: (lease: VaultLease) => Promise<T>,
	): Promise<T>;
	controller(deviceId: string): BrowserController | undefined;
	vault(deviceId: string): LocalDeviceVault | undefined;
	/** Verified at unlock; reused by fleet and live. */
	receipt(deviceId: string): DeviceReceipt | undefined;
	signer(deviceId: string): OwnerSigner | undefined;
	/** IA §6.4.1 per-user setting "Ask for my password again for access changes"; off by default. */
	askPasswordForAccessChanges(): boolean;
	/** Turning it on drops every held owner key in this window, so `canSign` turns false. */
	setAskPasswordForAccessChanges(ask: boolean): void;
}

/* Live sessions (M-DATA §3.3). */

export type RelayFallbackReason =
	| "no_turn_servers"
	| "ice_timeout"
	| "webrtc_failed"
	| "webrtc_unavailable";

export type LiveError =
	| {
			step: "getting_pass";
			code:
				| "not_configured"
				| "needs_wss"
				| "access_expired"
				| "epoch_mismatch"
				| "invalid_admission"
				| "http";
			status?: number;
	  }
	| { step: "reaching_device"; code: "relay_unreachable"; url?: string }
	| {
			step: "securing";
			code: "handshake_failed" | "identity_confirmation_failed";
	  }
	| {
			step: "reading_services";
			code: "rejected";
			rejection?: ManagementRejection;
	  }
	| { step: "session"; code: "closed" | "timeout" | "expired" };

export type LiveState =
	| { kind: "idle" }
	| { kind: "connecting"; step: UnlockStepId; startedAt: number }
	| {
			kind: "live";
			transport: "webrtc" | "websocket";
			fallbackReason?: RelayFallbackReason;
			expiresAt: number;
			bootId: string;
			connectedAt: number;
	  }
	| {
			kind: "renewing";
			transport: "webrtc" | "websocket";
			expiresAt: number;
	  }
	| {
			kind: "reconnecting";
			attempt: number;
			retryAt: number;
			cause: LiveError;
	  }
	| { kind: "unreachable"; retryAt: number; cause: LiveError }
	| { kind: "failed"; cause: LiveError };

export type CallLane = "user" | "operation" | "poll";

export interface DeviceCallOptions {
	lane?: CallLane;
	signal?: AbortSignal;
	/** Defaults to `READ_COMMANDS.has(command.type)`. Reads may be retried once after `ManagementUnconfirmedError`; writes never are. */
	idempotent?: boolean;
	trackUnconfirmed?: { kind: ActivityKind; target: ActivityTarget };
}

export interface LiveInspection {
	value: InspectionPlus;
	readAt: number;
	progress?: { pages: number };
	error?: LiveError;
}

export interface LiveSessionManager {
	state(deviceId: string): LiveState;
	subscribe(listener: () => void): () => void;
	/** Ref-counted demand: while > 0 and unlocked, keep a session open, renew and reconnect. */
	acquire(
		deviceId: string,
		reason: "view" | "operation" | "stream",
	): () => void;
	/** Bound to the device, not to a connection; plugs into every DM function unchanged. */
	call(deviceId: string, options?: DeviceCallOptions): ManagementCall;
	/** Multi-request critical section; polls wait, user calls queue behind it. */
	exclusive<T>(
		deviceId: string,
		run: (call: ManagementCall) => Promise<T>,
		options?: { signal?: AbortSignal },
	): Promise<T>;
	inspection(deviceId: string): LiveInspection | undefined;
	refreshInspection(deviceId: string): Promise<void>;
	close(deviceId: string): void;
	closeAll(): void;
}

export type StreamSpec =
	| { kind: "metrics"; placementId: string | null; everyMs?: number }
	| { kind: "project_metrics"; projectId: string; everyMs?: number }
	| {
			kind: "logs";
			placementId: string | null;
			follow: boolean;
			limit?: number;
	  }
	| {
			kind: "messages";
			placementId: string | null;
			projectId: string | null;
			limit?: number;
	  }
	| { kind: "offline_queues"; placementId: string; everyMs?: number }
	| { kind: "certificates"; everyMs?: number }
	| { kind: "group_metrics"; scope: string; everyMs?: number };

export interface StreamGap {
	kind: "evicted_through" | "truncated" | "dropped_lines" | "outbox_dropped";
	/** Last sequence or record number lost, when known. */
	through?: number;
	count?: number;
	at?: number;
}

export interface StreamState<T> {
	/** Never cleared by errors or renewals; cleared by lock. */
	data?: T;
	freshness: Freshness;
	gaps: StreamGap[];
	/** "Following · N behind". */
	behind?: number;
	/** Shown as "No access" / "Not supported", never "waiting…". */
	rejected?: ManagementRejection;
}

export interface LiveStreams {
	subscribe<T>(
		deviceId: string,
		spec: StreamSpec,
		listener: (state: StreamState<T>) => void,
	): () => void;
	loadOlder(
		deviceId: string,
		spec: Extract<StreamSpec, { kind: "logs" | "messages" }>,
	): Promise<void>;
}

/* Encrypted snapshots (M-DATA §3.4). */

/** A status snapshot's rows: the retained shape plus the agent facts a snapshot may carry (plan §3.4.2). */
export type StatusObservation = RetainedObservation &
	Partial<
		Pick<InspectionPlus, "agent" | "host" | "tasks" | "hostOperation">
	> & { placements: PlacementStatusPlus[] };

export interface FleetDeviceState {
	deviceId: string;
	status?: {
		observations: StatusObservation[];
		observedAt: number;
		bootId: string | null;
		sequence: number;
	};
	metrics?: FleetMetrics[];
	/** Saved inventory: "Snapshot · not live". */
	saved?: { observations: RetainedObservation[]; observedAt: number };
	reader?: { revision: number; expiresAt: number };
	/** The viewer's own grant from the verified policy (capabilities after unlock). */
	policy?: { version: number; myGrant?: ManagementGrant };
	previousBootId?: string;
	freshness: { status: Freshness; metrics: Freshness; saved: Freshness };
	error?: {
		kind: "network" | "integrity" | "access";
		message: string;
		at: number;
		retryAt?: number;
	};
}

export interface FleetSnapshotReader {
	get(deviceId: string): FleetDeviceState | undefined;
	subscribe(listener: () => void): () => void;
	/** Demand: polls while keys are unlocked. */
	watch(deviceId: string): () => void;
	refresh(deviceId: string): Promise<void>;
	/** "Renew": sign a new 365-day reader declaration now. Rejects without unlocked keys. */
	renew(deviceId: string): Promise<void>;
	/** "Stop receiving": remove this computer's reader on the hub; status reads "No access" until `renew`. */
	stopReceiving(deviceId: string): Promise<void>;
	coverage(projectId?: string): {
		readable: number;
		locked: number;
		noKeys: number;
		total: number;
	};
}

/* Activity tray (M-DATA §3.9). Never holds secrets. */

export type ActivityKind =
	| "safe_update"
	| "upload"
	| "access_rules"
	| "account_backup"
	| "agent_update"
	| "reboot"
	| "command"
	| "secret_write"
	| "history_readers"
	| "metric_readers"
	| "offline_write_retry"
	| "signing_request"
	| "setup";

export type ActivityState =
	| "active"
	| "paused"
	| "waiting"
	| "done"
	| "failed"
	| "unknown";

export type ActivityDetailCode =
	| "files_progress"
	| "bytes_progress"
	| "instances_progress"
	| "resumable_until"
	| "waiting_for_device"
	| "waiting_for_apply"
	| "waiting_for_heartbeat"
	| "waiting_for_backup"
	| "no_reply"
	| "failed"
	| "rolled_back"
	| "rejected"
	| "cancelled"
	| "done";

export type ActivityAction =
	| "open"
	| "check_again"
	| "activate"
	| "discard"
	| "resume"
	| "cancel"
	| "dismiss";

export interface ActivityTarget {
	deviceId: string;
	deviceName?: string;
	serviceId?: string;
	projectId?: string;
}

/** Times in a handle (`issuedAt`, `expiresAt`) are unix seconds, like the device's. */
export type ResumeHandle =
	| {
			type: "operation";
			operationId: string;
			command: string;
			issuedAt: number;
	  }
	| {
			type: "rollout";
			rolloutId: string;
			placementId: string;
			projectId: string;
	  }
	| {
			type: "transfer";
			transferId: string;
			projectId: string;
			manifestSha256: string;
			expiresAt: number;
			confirmed?: boolean;
	  }
	| {
			type: "host_operation";
			kind: "reboot" | "update_agent";
			operationId: string;
			bootIdBefore: string | null;
	  }
	| { type: "policy"; version: number }
	| { type: "account_backup"; revision: number }
	| {
			type: "secret";
			operationId: string;
			placementId: string;
			name: string;
	  }
	| { type: "csr"; requestId: string }
	| { type: "setup"; enrollmentId: string; deviceId?: string };

/** `deadlineAt`, `startedAt`, `updatedAt` and `finishedAt` are unix milliseconds on this computer's clock. */
export interface ActivityItem {
	id: string;
	kind: ActivityKind;
	target: ActivityTarget;
	state: ActivityState;
	label: CopyRef<ActivityKind>;
	detail?: CopyRef<ActivityDetailCode>;
	progress?:
		| {
				done: number;
				total: number;
				unit: "files" | "bytes" | "instances" | "seconds";
		  }
		| "indeterminate";
	deadlineAt?: number;
	startedAt: number;
	updatedAt: number;
	finishedAt?: number;
	startedBy: "you";
	resume?: ResumeHandle;
	actions: ActivityAction[];
	href?: NavTarget;
	dismissed?: boolean;
}

/** One deploy across several devices: one item per target, restored after a reload by its id. Times are unix milliseconds. */
export interface ActivityRun {
	id: string;
	title: CopyRef<string>;
	/** In target order. */
	itemIds: string[];
	oneAtATime: boolean;
	stopOnFail: boolean;
	startedAt: number;
	updatedAt: number;
}

export interface ActivityTracker {
	/** In progress · no reply · finished. */
	list(): ActivityItem[];
	subscribe(listener: () => void): () => void;
	start(item: Omit<ActivityItem, "id" | "startedAt" | "updatedAt">): string;
	startRun(run: {
		title: CopyRef<string>;
		oneAtATime: boolean;
		stopOnFail: boolean;
		items: Omit<ActivityItem, "id" | "startedAt" | "updatedAt">[];
	}): ActivityRun;
	run(runId: string): ActivityRun | undefined;
	/** Every item of the run still kept, dismissed ones included, in target order. */
	runItems(runId: string): ActivityItem[];
	runs(): ActivityRun[];
	update(id: string, patch: Partial<ActivityItem>): void;
	finish(
		id: string,
		outcome: "done" | "failed" | "unknown",
		detail?: ActivityItem["detail"],
	): void;
	dismiss(id: string): void;
	/** Re-read resumable items: keys (+ live) for operation/rollout/transfer/host_operation/secret/csr; hub only for policy/account_backup/setup. */
	resume(deviceId?: string): Promise<void>;
	onFinishedElsewhere(listener: (item: ActivityItem) => void): () => void;
}

/* This computer (M-DATA §3.10). */

export interface LocalVaultSummary {
	deviceId: string;
	role: "owner" | "shared";
	grantId: string;
	requiresFreshEndpoint: boolean;
	identityPinnedAt?: number;
	identityFingerprint?: string;
}

export interface LocalSummary {
	platform: "desktop" | "web";
	persistence: "persisted" | "denied" | "unavailable" | "unknown";
	webLocks: boolean;
	indexedDb: boolean;
	cryptoLoaded: boolean | "unknown";
	vaults: LocalVaultSummary[];
	backups: Record<
		string,
		{
			localRevision: number;
			pending: boolean;
			sourceDigestChanged?: boolean;
			/** The device password changed after the account backup was sealed (M11). */
			passwordChangedSinceBackup?: boolean;
		}
	>;
	authorities: {
		authorityId: string;
		label: string;
		issuingNotAfter: number;
		rootNotAfter: number;
	}[];
}

export interface LocalInventory {
	summary(): LocalSummary;
	subscribe(listener: () => void): () => void;
	/** After setup, import, restore or delete. */
	reload(): Promise<void>;
	/** D7 before the password. */
	identityCheck(
		deviceId: string,
		reported: DeviceIdentity,
	): "match" | "mismatch" | "unpinned";
	forgetIdentity(deviceId: string): Promise<void>;
	deleteKeys(deviceId: string): Promise<void>;
	/** Explicit "Keep keys safely". */
	requestPersistence(): Promise<LocalSummary["persistence"]>;
}

/* Clock (M-DATA §3.11). */

export interface ClockModel {
	/** Milliseconds, hub-corrected when an offset is known. */
	now(): number;
	/** This computer − hub, seconds (D9, clock_skew). */
	readonly hubOffsetS?: number;
	deviceSkewS(deviceId: string): number | undefined;
	/** For `snapshot`, `hubTimeS` is the device's `observed_at` and `localMs` the raw local arrival time; `deviceId` attributes it. */
	observe(
		source: "server_time" | "admission" | "snapshot",
		hubTimeS: number,
		localMs: number,
		deviceId?: string,
	): void;
}

/* Ports (CA5): factories receive these; only the registry wires concrete instances. */

export type KeyPort = Pick<
	KeySessionManager,
	"controller" | "vault" | "receipt" | "snapshot" | "subscribe"
> & { touch(deviceId: string): void };
export type LivePort = Pick<
	LiveSessionManager,
	"acquire" | "call" | "exclusive" | "state" | "close"
>;
export type FleetPort = Pick<FleetSnapshotReader, "watch" | "refresh" | "get">;
export type LocalPort = Pick<
	LocalInventory,
	"summary" | "reload" | "identityCheck"
>;
export interface HubPort {
	fetch<T>(
		path: string,
		init?: { method?: string; body?: unknown },
	): Promise<T>;
}
