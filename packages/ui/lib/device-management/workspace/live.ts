import type { ArtifactTransferStatus } from "../artifacts";
import {
	type InspectionReadOptions,
	readDeviceInspection,
} from "../inspection";
import { type InventoryWriter, createInventoryWriter } from "../inventory";
import type { InspectionPlus } from "../model/types";
import type { ModelAssetStatus } from "../models";
import type { ManagementCall } from "../telemetry";
import {
	type ConnectProgress,
	DeviceManagementConnection,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
	managementFailureDiagnostic,
} from "../transport";
import type { DeviceServiceStream, TunnelServiceOptions } from "../tunnel";
import {
	type DeviceTunnelDataClient,
	type TunnelArtifactUpload,
	type TunnelModelAssetPush,
	isTunnelReadCommand,
} from "../tunnel-data";
import {
	type BrowserController,
	type DeviceReceipt,
	type ManagementRejection,
	type ManagementResponse,
	managementRejection,
} from "../types";
import { admissionHubTimeS } from "./clock";
import {
	type DeviceErrorCode,
	LiveCallError,
	classifyDeviceError,
	fallbackCode,
	isFatalLiveError,
	liveErrorCode,
	toLiveError,
} from "./errors";
import type {
	ActivityTracker,
	CallLane,
	ClockModel,
	DeviceCallOptions,
	HubPort,
	KeyPort,
	LiveError,
	LiveInspection,
	LiveSessionManager,
	LiveState,
	RelayFallbackReason,
	StepDetailCode,
	UnlockStep,
	UnlockStepId,
	WorkspaceDeps,
} from "./types";

/** M-DATA §3.3 timings. Instants in `LiveState` and `LiveInspection.readAt` are unix seconds. */
export const LIVE_TIMING = {
	renewBeforeS: 45,
	replyWindowS: 15,
	minRenewAgeS: 15,
	backoffS: [1, 2, 5, 10, 30, 60],
	unreachableRetryS: 60,
	presenceCheckMs: 5_000,
	inspectionEveryMs: 20_000,
	lingerMs: 60_000,
} as const;

/** Reads may be retried once after an unconfirmed reply; everything else never is. */
export const READ_COMMANDS: ReadonlySet<string> = new Set([
	"service_listeners",
	"inspect_page",
	"inspect",
	"operation",
	"placement_configuration",
	"placement_config",
	"rollout",
	"metrics",
	"project_metrics",
	"logs",
	"messages",
	"telemetry_read",
	"telemetry_roster_read",
	"archive_roster_read",
	"archive_read",
	"offline_queue",
	"certificates",
	"certificate_requests",
	"certificate_issuers",
	"acme_certificates",
	"host_operation",
	"rollout_history",
	"operations",
	"metrics_history",
	"offline_queue_operations",
	"offline_queue_lookup",
]);

/** `models` carries reads and writes; these request kinds only read. */
const MODELS_READ_KINDS: ReadonlySet<string> = new Set([
	"overview",
	"models",
	"jobs",
	"recommendations",
	"probe",
	"stats",
]);

/** A read: `READ_COMMANDS`, or a `models` request of a read kind. */
export function isReadCommand(command: Record<string, unknown>): boolean {
	if (READ_COMMANDS.has(String(command.type))) return true;
	const request = command.request as Record<string, unknown> | undefined;
	return (
		command.type === "models" && MODELS_READ_KINDS.has(String(request?.kind))
	);
}

/** User commands after which the inspection is read again. */
const STATE_CHANGING: ReadonlySet<string> = new Set([
	"apply",
	"stage_rollout",
	"activate_rollout",
	"cancel_rollout",
	"rollout_secret",
	"set_secret",
	"scale",
	"start",
	"stop",
	"restart",
	"remove",
]);

const LANES: readonly CallLane[] = ["user", "operation", "poll"];
/** Only an owner's admission lasts exactly 300 s; a grantee's may end with the grant. */
const OWNER_GRANT = "owner";
const LIVE_STEPS: readonly UnlockStepId[] = [
	"getting_pass",
	"reaching_device",
	"trying_direct",
	"securing",
	"reading_services",
];

export interface LiveConnection {
	readonly transport: "webrtc" | "websocket";
	readonly fallbackReason?: RelayFallbackReason;
	/** Unix seconds, device-confirmed. */
	readonly expiresAt: number;
	readonly bootId: string;
	readonly open: boolean;
	request(
		command: Record<string, unknown>,
		operationId?: string,
	): Promise<ManagementResponse>;
	requestData?(
		command: Record<string, unknown>,
		operationId?: string,
		signal?: AbortSignal,
	): Promise<ManagementResponse>;
	uploadArtifact?(input: TunnelArtifactUpload): Promise<ArtifactTransferStatus>;
	pushModelAsset?(input: TunnelModelAssetPush): Promise<ModelAssetStatus>;
	openService?(
		placementId: string,
		serviceId: string,
		options?: TunnelServiceOptions,
	): Promise<DeviceServiceStream>;
	openModelGateway?(options?: {
		signal?: AbortSignal;
	}): Promise<DeviceServiceStream>;
	detachDataTunnel?(): DeviceTunnelDataClient | undefined;
	adoptDataTunnel?(client: DeviceTunnelDataClient): void;
	close(): void;
	onClosed(listener: (reason: "local" | "remote") => void): () => void;
}

export interface LiveConnectInput {
	deviceId: string;
	controller: BrowserController;
	receipt: DeviceReceipt;
	grantId: string;
	signal: AbortSignal;
	onStep: (progress: ConnectProgress) => void;
}

export interface LivePorts {
	keys: KeyPort;
	hub: HubPort;
	clock: ClockModel;
	activity: Pick<ActivityTracker, "start" | "finish">;
	presence: (
		deviceId: string,
	) => "online" | "late" | "offline" | "never" | "revoked";
}

/** Seams for tests and the registry; production uses the defaults. */
export interface LiveManagerOptions {
	connect?: (input: LiveConnectInput) => Promise<LiveConnection>;
	readInspection?: (
		call: ManagementCall,
		deviceId: string,
		options: InspectionReadOptions,
	) => Promise<InspectionPlus>;
	inventoryWriter?: (
		controller: BrowserController,
		grantId: string,
		active: () => boolean,
	) => Promise<InventoryWriter>;
	schedule?: (run: () => void, ms: number) => () => void;
	/** Each completed inspection (feeds the fleet view so it prefers live). */
	onInspection?: (deviceId: string, inspection: LiveInspection) => void;
}

export interface LiveSessionManagerImpl extends LiveSessionManager {
	/** `lane` defaults to "operation"; background sections (group metrics) run on "poll". */
	exclusive<T>(
		deviceId: string,
		run: (call: ManagementCall) => Promise<T>,
		options?: { signal?: AbortSignal; lane?: CallLane },
	): Promise<T>;
	/** Progress of the latest connection attempt (unlock sheet, Copy diagnostics). */
	steps(deviceId: string): UnlockStep[];
	/** Retry an unreachable device at once when its presence turns online. */
	presenceChanged(deviceId?: string): void;
	/** Every device the manager has seen (streams use it to resume and auto-load). */
	devices(): string[];
	dispose(): void;
}

type Reason = "view" | "operation" | "stream";

interface RequestTask {
	kind: "request";
	generation: number;
	lane: CallLane;
	command: Record<string, unknown>;
	operationId?: string;
	idempotent: boolean;
	retried: boolean;
	signal?: AbortSignal;
	track?: DeviceCallOptions["trackUnconfirmed"];
	coalesceKey?: string;
	promise: Promise<ManagementResponse>;
	resolve: (response: ManagementResponse) => void;
	reject: (error: unknown) => void;
}

interface ExclusiveTask {
	kind: "exclusive";
	generation: number;
	lane: CallLane;
	signal?: AbortSignal;
	run: (call: ManagementCall) => Promise<void>;
	reject: (error: unknown) => void;
}

type Task = RequestTask | ExclusiveTask;
type TimerName = "retry" | "renew" | "presence" | "inspect" | "linger";

interface Entry {
	id: string;
	generation: number;
	demand: Record<Reason, number>;
	state: LiveState;
	conn?: LiveConnection;
	connectedAtS: number;
	controller?: BrowserController;
	/** The key session last seen for this device; a new one serves demand taken while locked. */
	keys?: BrowserController;
	connecting?: Promise<LiveConnection>;
	abort?: AbortController;
	attempt: number;
	queues: Record<CallLane, Task[]>;
	coalesced: Map<string, RequestTask>;
	draining: boolean;
	bulkActive: number;
	dataTunnel?: DeviceTunnelDataClient;
	timers: Partial<Record<TimerName, () => void>>;
	inspection?: LiveInspection;
	inspecting?: Promise<void>;
	inspectAgain: boolean;
	writer?: InventoryWriter;
	writerController?: BrowserController;
	steps: UnlockStep[];
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((done, fail) => {
		resolve = done;
		reject = fail;
	});
	return { promise, resolve, reject };
}

function abortError(signal: AbortSignal): unknown {
	return signal.reason ?? new DOMException("Aborted", "AbortError");
}

const STEP_DETAILS = new Set<string>([
	"not_configured",
	"needs_wss",
	"access_expired",
	"epoch_mismatch",
	"invalid_admission",
	"http_error",
	"relay_unreachable",
	"handshake_failed",
	"identity_confirmation_failed",
	"session_closed",
	"timeout",
	"expired",
	"cancelled",
	"slots_in_use",
]);

function stepDetail(code: DeviceErrorCode): StepDetailCode | undefined {
	if (code.startsWith("rejected")) return "rejected";
	return STEP_DETAILS.has(code) ? (code as StepDetailCode) : undefined;
}

function freshSteps(): UnlockStep[] {
	return LIVE_STEPS.map((id) => ({ id, state: "pending" }));
}

function totalDemand(entry: Entry): number {
	return (
		entry.demand.view +
		entry.demand.operation +
		entry.demand.stream +
		entry.bulkActive
	);
}

function hasTasks(entry: Entry): boolean {
	return LANES.some((lane) => entry.queues[lane].length > 0);
}

class LiveManager implements LiveSessionManagerImpl {
	private readonly entries = new Map<string, Entry>();
	private readonly listeners = new Set<() => void>();
	private readonly stopKeys: () => void;
	private readonly nowMs: () => number;
	private readonly schedule: (run: () => void, ms: number) => () => void;
	private readonly connector: (
		input: LiveConnectInput,
	) => Promise<LiveConnection>;
	private readonly readInspection: NonNullable<
		LiveManagerOptions["readInspection"]
	>;
	private readonly writerFactory: NonNullable<
		LiveManagerOptions["inventoryWriter"]
	>;

	constructor(
		private readonly deps: WorkspaceDeps,
		private readonly ports: LivePorts,
		private readonly options: LiveManagerOptions,
	) {
		this.nowMs = deps.now ?? Date.now;
		this.schedule =
			options.schedule ??
			((run, ms) => {
				const timer = setTimeout(run, ms);
				return () => clearTimeout(timer);
			});
		this.connector =
			options.connect ??
			((input) =>
				DeviceManagementConnection.connect(
					deps.api,
					deps.profile,
					input.controller,
					input.receipt,
					input.grantId,
					input.signal,
					{ onStep: input.onStep },
				));
		this.readInspection = options.readInspection ?? readDeviceInspection;
		this.writerFactory =
			options.inventoryWriter ??
			((controller, grantId, active) =>
				createInventoryWriter(
					deps.api,
					deps.profile,
					deps.scope,
					controller,
					grantId,
					active,
				));
		this.stopKeys = ports.keys.subscribe(() => this.keysChanged());
	}

	/* Contract surface. */

	state(deviceId: string): LiveState {
		return this.entries.get(deviceId)?.state ?? { kind: "idle" };
	}

	subscribe(listener: () => void): () => void {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}

	steps(deviceId: string): UnlockStep[] {
		return this.entries.get(deviceId)?.steps ?? freshSteps();
	}

	acquire(deviceId: string, reason: Reason): () => void {
		const entry = this.entry(deviceId);
		entry.demand[reason]++;
		this.cancel(entry, "linger");
		this.ensure(entry);
		if (reason === "view") this.startInspectTimer(entry);
		let released = false;
		return () => {
			if (released) return;
			released = true;
			entry.demand[reason]--;
			this.settle(entry);
		};
	}

	call(deviceId: string, options: DeviceCallOptions = {}): ManagementCall {
		return (command, operationId) =>
			this.request(deviceId, command, operationId, options);
	}
	openService(
		deviceId: string,
		placementId: string,
		serviceId = "hosting",
		options: TunnelServiceOptions = {},
	): Promise<DeviceServiceStream> {
		return this.openStream(deviceId, options.signal, (connection) => {
			if (!connection.openService)
				throw new Error("Update Studio to connect to deployed services.");
			return connection.openService(placementId, serviceId, options);
		});
	}
	/** Only for agents with `model_host`: an older one closes the whole tunnel on the unknown target. */
	openModelGateway(
		deviceId: string,
		options: { signal?: AbortSignal } = {},
	): Promise<DeviceServiceStream> {
		const entry = this.entry(deviceId);
		return this.openStream(deviceId, options.signal, (connection) => {
			if (!connection.openModelGateway)
				throw new Error("Update Studio to send requests to models on devices.");
			const features = entry.inspection?.value.features;
			if (features && features.model_host !== 1)
				throw new LiveCallError(
					"rejected_unsupported",
					"Update the device agent to send requests to its models.",
				);
			return connection.openModelGateway(options);
		});
	}
	/** One stream of the session; its demand holds the session until the stream closes. */
	private async openStream(
		deviceId: string,
		signal: AbortSignal | undefined,
		open: (connection: LiveConnection) => Promise<DeviceServiceStream>,
	): Promise<DeviceServiceStream> {
		const entry = this.entry(deviceId);
		const refused = this.refusal(entry);
		if (refused) throw refused;
		signal?.throwIfAborted();
		const generation = entry.generation;
		const release = this.acquire(deviceId, "stream");
		let stream: DeviceServiceStream | undefined;
		try {
			const connection = await this.sessionFor(entry);
			if (generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			signal?.throwIfAborted();
			stream = await open(connection);
			if (generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			signal?.throwIfAborted();
			void stream.closed.then(release);
			return stream;
		} catch (error) {
			stream?.reset();
			release();
			throw error;
		}
	}
	uploadArtifact(
		deviceId: string,
		input: TunnelArtifactUpload,
	): Promise<ArtifactTransferStatus> {
		return this.transfer(deviceId, input.signal, (connection) => {
			if (!connection.uploadArtifact)
				throw new Error(
					"This device connection does not support streamed uploads.",
				);
			return connection.uploadArtifact(input);
		});
	}
	/** One model asset push, or the probe that opens its push session (plan §3.1). */
	pushModelAsset(
		deviceId: string,
		input: TunnelModelAssetPush,
	): Promise<ModelAssetStatus> {
		return this.transfer(deviceId, input.signal, (connection) => {
			if (!connection.pushModelAsset)
				throw new Error(
					"This device connection does not support streamed uploads.",
				);
			return connection.pushModelAsset(input);
		});
	}
	/** A transfer over the session's data tunnel; its demand holds the session until it ends. */
	private async transfer<T>(
		deviceId: string,
		signal: AbortSignal | undefined,
		run: (connection: LiveConnection) => Promise<T>,
	): Promise<T> {
		const entry = this.entry(deviceId);
		const generation = entry.generation;
		const refused = this.refusal(entry);
		if (refused) throw refused;
		if (signal?.aborted) throw abortError(signal);
		const release = this.acquire(deviceId, "stream");
		try {
			const connection = await this.sessionFor(entry);
			if (generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			const result = await run(connection);
			if (generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			return result;
		} finally {
			release();
		}
	}

	exclusive<T>(
		deviceId: string,
		run: (call: ManagementCall) => Promise<T>,
		options: { signal?: AbortSignal; lane?: CallLane } = {},
	): Promise<T> {
		const entry = this.entry(deviceId);
		const refused = this.refusal(entry);
		if (refused) return Promise.reject(refused);
		const result = deferred<T>();
		this.enqueue(entry, {
			kind: "exclusive",
			generation: entry.generation,
			lane: options.lane ?? "operation",
			signal: options.signal,
			reject: result.reject,
			run: (call) => run(call).then(result.resolve, result.reject),
		});
		return result.promise;
	}

	inspection(deviceId: string): LiveInspection | undefined {
		return this.entries.get(deviceId)?.inspection;
	}

	refreshInspection(deviceId: string): Promise<void> {
		return this.inspect(this.entry(deviceId), "operation");
	}

	close(deviceId: string): void {
		const entry = this.entries.get(deviceId);
		if (!entry) return;
		this.shutdown(
			entry,
			new LiveCallError("session_closed", "The live session was closed."),
		);
	}

	closeAll(): void {
		for (const id of this.entries.keys()) this.close(id);
	}

	devices(): string[] {
		return [...this.entries.keys()];
	}

	presenceChanged(deviceId?: string): void {
		for (const entry of this.entries.values())
			if (
				(!deviceId || entry.id === deviceId) &&
				entry.state.kind === "unreachable" &&
				this.reachable(entry.id)
			) {
				this.cancel(entry, "retry");
				this.ensure(entry);
			}
	}

	dispose(): void {
		this.stopKeys();
		this.closeAll();
		for (const entry of this.entries.values()) this.cancel(entry, "inspect");
		this.listeners.clear();
	}

	/* Entries and timers. */

	private entry(deviceId: string): Entry {
		let entry = this.entries.get(deviceId);
		if (!entry) {
			entry = {
				id: deviceId,
				generation: 0,
				demand: { view: 0, operation: 0, stream: 0 },
				state: { kind: "idle" },
				connectedAtS: 0,
				keys: this.ports.keys.controller(deviceId),
				attempt: 0,
				queues: { user: [], operation: [], poll: [] },
				coalesced: new Map(),
				draining: false,
				bulkActive: 0,
				timers: {},
				inspectAgain: false,
				steps: freshSteps(),
			};
			this.entries.set(deviceId, entry);
		}
		return entry;
	}

	private notify(): void {
		for (const listener of this.listeners) listener();
	}

	private setState(entry: Entry, state: LiveState): void {
		entry.state = state;
		this.notify();
	}

	private nowS(): number {
		return Math.floor(this.nowMs() / 1000);
	}

	private timer(entry: Entry, name: TimerName, ms: number, run: () => void) {
		this.cancel(entry, name);
		entry.timers[name] = this.schedule(() => {
			entry.timers[name] = undefined;
			run();
		}, ms);
	}

	private cancel(entry: Entry, name: TimerName): void {
		entry.timers[name]?.();
		entry.timers[name] = undefined;
	}

	private reachable(deviceId: string): boolean {
		const presence = this.ports.presence(deviceId);
		return presence === "online" || presence === "late";
	}

	private markStep(
		entry: Entry,
		id: UnlockStepId,
		state: UnlockStep["state"],
		detail?: StepDetailCode,
	): void {
		entry.steps = entry.steps.map((step) =>
			step.id === id
				? { id, state, ...(detail ? { detail: { code: detail } } : {}) }
				: step,
		);
		this.notify();
	}

	/* Demand: connect while needed, linger 60 s after. */

	private ensure(entry: Entry): void {
		if (
			totalDemand(entry) === 0 ||
			entry.conn?.open ||
			entry.connecting ||
			entry.timers.retry ||
			entry.state.kind === "failed"
		)
			return;
		this.connect(entry).catch(() => {});
	}

	private settle(entry: Entry): void {
		if (totalDemand(entry) > 0 || hasTasks(entry) || entry.draining) return;
		entry.dataTunnel?.close();
		entry.dataTunnel = undefined;
		if (
			entry.state.kind === "reconnecting" ||
			entry.state.kind === "unreachable"
		) {
			this.cancel(entry, "retry");
			this.cancel(entry, "presence");
			this.setState(entry, { kind: "idle" });
			return;
		}
		if (!entry.conn && !entry.connecting) return;
		if (entry.timers.linger) return;
		this.timer(entry, "linger", LIVE_TIMING.lingerMs, () => {
			if (totalDemand(entry) > 0 || hasTasks(entry) || entry.draining) return;
			entry.abort?.abort();
			this.closeConnection(entry);
			this.cancel(entry, "renew");
			this.setState(entry, { kind: "idle" });
		});
	}

	private startInspectTimer(entry: Entry): void {
		if (entry.timers.inspect) return;
		this.timer(entry, "inspect", LIVE_TIMING.inspectionEveryMs, () => {
			if (entry.demand.view === 0) return;
			if (entry.state.kind === "live")
				this.inspect(entry, "poll").catch(() => {});
			this.startInspectTimer(entry);
		});
	}

	/* Connecting, renewal and reconnect. */

	private connect(entry: Entry): Promise<LiveConnection> {
		if (entry.connecting) return entry.connecting;
		const { keys } = this.ports;
		const controller = keys.controller(entry.id);
		const vault = keys.vault(entry.id);
		const receipt = keys.receipt(entry.id);
		if (!controller || !vault || !receipt) {
			if (entry.state.kind !== "idle") this.setState(entry, { kind: "idle" });
			return Promise.reject(
				new LiveCallError(
					"keys_locked",
					"Unlock this device before connecting.",
				),
			);
		}
		this.cancel(entry, "retry");
		this.cancel(entry, "presence");
		this.closeConnection(entry);
		const renewing = entry.state.kind === "renewing";
		const abort = new AbortController();
		entry.abort = abort;
		entry.steps = freshSteps();
		if (!renewing)
			this.setState(entry, {
				kind: "connecting",
				step: "getting_pass",
				startedAt: this.nowS(),
			});
		const attempt = this.connector({
			deviceId: entry.id,
			controller,
			receipt,
			grantId: vault.grantId,
			signal: abort.signal,
			onStep: (progress) => {
				if (!abort.signal.aborted)
					this.onStep(entry, progress, renewing, vault.grantId);
			},
		})
			.then((conn) => this.opened(entry, conn, controller, vault.grantId))
			.catch((error: unknown) => {
				throw this.connectFailed(entry, error);
			})
			.finally(() => {
				if (entry.connecting === attempt) entry.connecting = undefined;
			});
		entry.connecting = attempt;
		return attempt;
	}

	private onStep(
		entry: Entry,
		progress: ConnectProgress,
		renewing: boolean,
		grantId: string,
	): void {
		if (progress.admission && grantId === OWNER_GRANT)
			this.ports.clock.observe(
				"admission",
				admissionHubTimeS(progress.admission.expiresAt),
				progress.admission.receivedAtMs,
			);
		const detail = progress.fallbackReason
			? (fallbackCode(progress.fallbackReason) as StepDetailCode)
			: undefined;
		this.markStep(entry, progress.step, progress.state, detail);
		if (!renewing && progress.state === "active")
			this.setState(entry, {
				kind: "connecting",
				step: progress.step,
				startedAt:
					entry.state.kind === "connecting"
						? entry.state.startedAt
						: this.nowS(),
			});
	}

	private opened(
		entry: Entry,
		conn: LiveConnection,
		controller: BrowserController,
		grantId: string,
	): LiveConnection {
		if (
			entry.abort?.signal.aborted ||
			this.ports.keys.controller(entry.id) !== controller ||
			this.ports.keys.vault(entry.id)?.grantId !== grantId
		) {
			conn.close();
			throw new LiveCallError(
				"keys_locked",
				"Device access changed while connecting.",
			);
		}
		entry.conn = conn;
		if (entry.dataTunnel) {
			if (conn.adoptDataTunnel) conn.adoptDataTunnel(entry.dataTunnel);
			else entry.dataTunnel.close();
			entry.dataTunnel = undefined;
		}
		entry.controller = controller;
		entry.connectedAtS = this.nowS();
		entry.attempt = 0;
		conn.onClosed((reason) => this.closed(entry, conn, reason));
		this.setState(entry, {
			kind: "live",
			transport: conn.transport,
			...(conn.fallbackReason ? { fallbackReason: conn.fallbackReason } : {}),
			expiresAt: conn.expiresAt,
			bootId: conn.bootId,
			connectedAt: entry.connectedAtS,
		});
		this.scheduleRenewal(entry, conn);
		if (entry.demand.view > 0) this.startInspectTimer(entry);
		this.startSession(entry, controller, grantId).catch(() => {});
		return conn;
	}

	private async startSession(
		entry: Entry,
		controller: BrowserController,
		grantId: string,
	): Promise<void> {
		const generation = entry.generation;
		if (entry.writerController !== controller) {
			entry.writerController = controller;
			entry.writer = undefined;
			const writer = await this.writerFactory(
				controller,
				grantId,
				() => this.ports.keys.controller(entry.id) === controller,
			).catch(() => undefined);
			if (entry.writerController === controller) {
				entry.writer = writer;
				if (!writer) entry.writerController = undefined;
			}
		}
		if (
			entry.generation !== generation ||
			!entry.conn?.open ||
			entry.controller !== controller
		)
			return;
		this.markStep(entry, "reading_services", "active");
		await this.inspect(entry, "operation");
	}

	/** Saving is best effort: a write that fails ends its writer, and the next session prepares a new one. */
	private retain(entry: Entry, value: InspectionPlus): void {
		const writer = entry.writer;
		if (!writer) return;
		const drop = () => {
			if (entry.writer !== writer) return;
			entry.writer = undefined;
			entry.writerController = undefined;
		};
		try {
			writer(value).catch(drop);
		} catch {
			drop();
		}
	}

	private connectFailed(entry: Entry, error: unknown): unknown {
		const cause = toLiveError(error);
		const failure = classifyDeviceError(error);
		const step = LIVE_STEPS.find(
			(id) => entry.steps.find((row) => row.id === id)?.state === "active",
		);
		if (step) this.markStep(entry, step, "failed", stepDetail(failure.code));
		if (error instanceof LiveCallError) return error;
		if (!cause) {
			entry.dataTunnel?.close();
			entry.dataTunnel = undefined;
			this.setState(entry, { kind: "idle" });
			return new LiveCallError("cancelled", failure.message);
		}
		if (isFatalLiveError(cause)) {
			entry.dataTunnel?.close();
			entry.dataTunnel = undefined;
			this.setState(entry, { kind: "failed", cause });
		} else if (totalDemand(entry) > 0 || hasTasks(entry))
			this.scheduleRetry(entry, cause);
		else {
			entry.dataTunnel?.close();
			entry.dataTunnel = undefined;
			this.setState(entry, { kind: "idle" });
		}
		return new LiveCallError(
			liveErrorCode(cause),
			failure.message,
			cause,
			managementFailureDiagnostic(error),
		);
	}

	private scheduleRetry(entry: Entry, cause: LiveError): void {
		if (!this.reachable(entry.id)) {
			const delayS = LIVE_TIMING.unreachableRetryS;
			this.setState(entry, {
				kind: "unreachable",
				retryAt: this.nowS() + delayS,
				cause,
			});
			this.timer(entry, "retry", delayS * 1000, () => this.ensure(entry));
			this.watchPresence(entry);
			return;
		}
		entry.attempt++;
		const { backoffS } = LIVE_TIMING;
		const delayS = backoffS[Math.min(entry.attempt, backoffS.length) - 1];
		this.setState(entry, {
			kind: "reconnecting",
			attempt: entry.attempt,
			retryAt: this.nowS() + delayS,
			cause,
		});
		this.timer(entry, "retry", delayS * 1000, () => this.ensure(entry));
	}

	private watchPresence(entry: Entry): void {
		this.timer(entry, "presence", LIVE_TIMING.presenceCheckMs, () => {
			if (entry.state.kind !== "unreachable") return;
			if (this.reachable(entry.id)) this.presenceChanged(entry.id);
			else this.watchPresence(entry);
		});
	}

	private scheduleRenewal(entry: Entry, conn: LiveConnection): void {
		const dueS = Math.max(
			LIVE_TIMING.minRenewAgeS,
			conn.expiresAt - LIVE_TIMING.renewBeforeS - this.nowS(),
		);
		this.timer(entry, "renew", dueS * 1000, () => {
			if (entry.conn !== conn || entry.draining || totalDemand(entry) === 0)
				return;
			this.renew(entry).catch(() => {});
		});
	}

	/** Break-then-make: each grant has only two connection slots (IA §3.2, D12). */
	private async renew(entry: Entry): Promise<LiveConnection> {
		const conn = entry.conn;
		const data = conn?.detachDataTunnel?.();
		if (data) entry.dataTunnel = data;
		if (conn)
			this.setState(entry, {
				kind: "renewing",
				transport: conn.transport,
				expiresAt: conn.expiresAt,
			});
		this.closeConnection(entry);
		return this.connect(entry);
	}

	/** The connection stops being the entry's before it closes, so its close event is not taken for a drop. */
	private closeConnection(entry: Entry): void {
		const conn = entry.conn;
		entry.conn = undefined;
		this.cancel(entry, "renew");
		conn?.close();
	}

	private closed(
		entry: Entry,
		conn: LiveConnection,
		reason: "local" | "remote",
	): void {
		if (entry.conn !== conn) return;
		entry.conn = undefined;
		this.cancel(entry, "renew");
		if (
			(totalDemand(entry) > 0 || hasTasks(entry)) &&
			this.ports.keys.controller(entry.id)
		) {
			if (reason === "remote") entry.dataTunnel = conn.detachDataTunnel?.();
			this.scheduleRetry(entry, { step: "session", code: "closed" });
		} else this.setState(entry, { kind: "idle" });
	}

	/** A session with room for one full reply window; renews or connects when there is none. */
	private async sessionFor(entry: Entry): Promise<LiveConnection> {
		for (let round = 0; round < 3; round++) {
			const conn = entry.conn;
			if (conn?.open) {
				const remaining = conn.expiresAt - this.nowS();
				const age = this.nowS() - entry.connectedAtS;
				if (
					remaining > LIVE_TIMING.renewBeforeS ||
					(remaining > LIVE_TIMING.replyWindowS + 1 &&
						age < LIVE_TIMING.minRenewAgeS)
				)
					return conn;
				await this.renew(entry);
				continue;
			}
			if (conn) this.closeConnection(entry);
			if (entry.state.kind === "failed")
				throw new LiveCallError(
					liveErrorCode(entry.state.cause),
					"The live session needs your attention before it can reconnect.",
					entry.state.cause,
				);
			await this.connect(entry);
		}
		throw new LiveCallError(
			"session_closed",
			"The device session could not be kept open.",
		);
	}

	/* The request queue: one request in flight per device, user > operation > poll. */

	private refusal(entry: Entry): LiveCallError | undefined {
		if (entry.state.kind === "failed")
			return new LiveCallError(
				liveErrorCode(entry.state.cause),
				"The live session needs your attention before it can reconnect.",
				entry.state.cause,
			);
		if (!this.ports.keys.controller(entry.id))
			return new LiveCallError("keys_locked", "Unlock this device first.");
		return undefined;
	}

	private request(
		deviceId: string,
		command: Record<string, unknown>,
		operationId: string | undefined,
		options: DeviceCallOptions,
	): Promise<ManagementResponse> {
		const entry = this.entry(deviceId);
		const refused = this.refusal(entry);
		if (refused) return Promise.reject(refused);
		if (options.signal?.aborted)
			return Promise.reject(abortError(options.signal));
		const lane = options.lane ?? "user";
		const coalesceKey =
			lane === "poll" && !operationId ? JSON.stringify(command) : undefined;
		const queued = coalesceKey && entry.coalesced.get(coalesceKey);
		if (queued) return queued.promise;
		const result = deferred<ManagementResponse>();
		const task: RequestTask = {
			kind: "request",
			generation: entry.generation,
			lane,
			command,
			operationId,
			idempotent: options.idempotent ?? isReadCommand(command),
			retried: false,
			signal: options.signal,
			track: options.trackUnconfirmed,
			coalesceKey,
			...result,
		};
		if (coalesceKey) entry.coalesced.set(coalesceKey, task);
		this.enqueue(entry, task);
		return result.promise;
	}

	private enqueue(entry: Entry, task: Task): void {
		entry.queues[task.lane].push(task);
		this.cancel(entry, "linger");
		this.drain(entry).catch(() => {});
	}

	private dequeue(entry: Entry): Task | undefined {
		const exclusiveWaiting = LANES.some((lane) =>
			entry.queues[lane].some((task) => task.kind === "exclusive"),
		);
		for (const lane of LANES) {
			const queue = entry.queues[lane];
			while (queue.length) {
				const index = queue.findIndex((task) => {
					if (task.kind === "exclusive") return entry.bulkActive === 0;
					if (!entry.conn?.requestData || !isTunnelReadCommand(task.command))
						return true;
					return !exclusiveWaiting && entry.bulkActive < 8;
				});
				if (index < 0) break;
				const task = queue.splice(index, 1)[0] as Task;
				if (task.kind === "request" && task.coalesceKey)
					entry.coalesced.delete(task.coalesceKey);
				if (task.signal?.aborted) {
					task.reject(abortError(task.signal));
					continue;
				}
				return task;
			}
		}
		return undefined;
	}

	private rejectQueued(entry: Entry, error: unknown): void {
		for (const lane of LANES) {
			const queue = entry.queues[lane].splice(0);
			for (const task of queue) task.reject(error);
		}
		entry.coalesced.clear();
	}

	private async drain(entry: Entry): Promise<void> {
		if (entry.draining) return;
		entry.draining = true;
		try {
			while (hasTasks(entry)) {
				try {
					await this.sessionFor(entry);
				} catch (error) {
					this.rejectQueued(entry, error);
					break;
				}
				const task = this.dequeue(entry);
				if (!task) break;
				if (task.kind === "exclusive") await this.runExclusive(entry, task);
				else if (entry.conn?.requestData && isTunnelReadCommand(task.command)) {
					entry.bulkActive++;
					void this.runRequest(entry, task).finally(() => {
						entry.bulkActive--;
						if (!entry.draining) this.afterDrain(entry);
						if (hasTasks(entry)) this.drain(entry).catch(() => {});
					});
				} else await this.runRequest(entry, task);
			}
		} finally {
			entry.draining = false;
			this.afterDrain(entry);
		}
	}

	private afterDrain(entry: Entry): void {
		const conn = entry.conn;
		if (
			conn?.open &&
			totalDemand(entry) > 0 &&
			conn.expiresAt - this.nowS() <= LIVE_TIMING.renewBeforeS &&
			this.nowS() - entry.connectedAtS >= LIVE_TIMING.minRenewAgeS
		)
			this.renew(entry).catch(() => {});
		this.settle(entry);
	}

	private async runRequest(entry: Entry, task: RequestTask): Promise<void> {
		try {
			const response = await this.transmit(entry, task);
			task.resolve(response);
			if (
				task.lane === "user" &&
				response.state === "completed" &&
				STATE_CHANGING.has(String(task.command.type))
			)
				this.inspect(entry, "operation").catch(() => {});
		} catch (error) {
			task.reject(error);
		}
	}

	private async runExclusive(entry: Entry, task: ExclusiveTask): Promise<void> {
		let chain: Promise<unknown> = Promise.resolve();
		const call: ManagementCall = (command, operationId) => {
			const next = chain.then(() =>
				this.transmit(entry, {
					generation: task.generation,
					signal: task.signal,
					lane: task.lane,
					command,
					operationId,
					idempotent: isReadCommand(command),
					retried: false,
				}),
			);
			chain = next.catch(() => {});
			return next;
		};
		try {
			await task.run(call);
		} catch (error) {
			task.reject(error);
		}
	}

	/**
	 * Sends one request; NotSent and pre-send expiry retry once on the next session, reads also after no reply.
	 * A change that got no reply becomes a tray item, unless background polling sent it: nobody asked for that one.
	 */
	private async transmit(
		entry: Entry,
		task: Pick<
			RequestTask,
			| "lane"
			| "command"
			| "operationId"
			| "idempotent"
			| "retried"
			| "track"
			| "signal"
			| "generation"
		>,
	): Promise<ManagementResponse> {
		for (;;) {
			if (task.generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			if (task.signal?.aborted) throw abortError(task.signal);
			const conn = await this.sessionFor(entry);
			if (task.generation !== entry.generation)
				throw new LiveCallError(
					"session_closed",
					"The live session was closed.",
				);
			try {
				const response = await (conn.requestData &&
				isTunnelReadCommand(task.command)
					? conn.requestData(task.command, task.operationId, task.signal)
					: conn.request(task.command, task.operationId));
				if (task.generation !== entry.generation)
					throw new LiveCallError(
						"session_closed",
						"The live session was closed.",
					);
				return response;
			} catch (error) {
				if (!task.retried && this.retryable(conn, task, error)) {
					task.retried = true;
					continue;
				}
				if (
					error instanceof ManagementUnconfirmedError &&
					!task.idempotent &&
					task.lane !== "poll"
				)
					this.recordUnknown(entry, task, error);
				throw error;
			}
		}
	}

	private retryable(
		conn: LiveConnection,
		task: Pick<RequestTask, "idempotent">,
		error: unknown,
	): boolean {
		if (error instanceof ManagementRequestNotSentError) return !conn.open;
		if (error instanceof ManagementUnconfirmedError) return task.idempotent;
		return !conn.open;
	}

	private recordUnknown(
		entry: Entry,
		task: Pick<RequestTask, "command" | "track">,
		error: ManagementUnconfirmedError,
	): void {
		const kind = task.track?.kind ?? "command";
		const id = this.ports.activity.start({
			kind,
			target: task.track?.target ?? { deviceId: entry.id },
			state: "unknown",
			label: { code: kind },
			detail: { code: "no_reply" },
			startedBy: "you",
			resume: {
				type: "operation",
				operationId: error.operationId,
				command: String(task.command.type),
				issuedAt: this.nowS(),
			},
			actions: ["check_again", "dismiss"],
		});
		this.ports.activity.finish(id, "unknown", { code: "no_reply" });
	}

	/* Inspection cache. */

	private inspect(entry: Entry, lane: CallLane): Promise<void> {
		if (entry.inspecting) {
			entry.inspectAgain = true;
			return entry.inspecting;
		}
		const run = (async () => {
			do {
				entry.inspectAgain = false;
				await this.readInspectionOnce(entry, lane);
			} while (entry.inspectAgain);
		})().finally(() => {
			entry.inspecting = undefined;
		});
		entry.inspecting = run;
		return run;
	}

	private async readInspectionOnce(
		entry: Entry,
		lane: CallLane,
	): Promise<void> {
		const generation = entry.generation;
		let rejection: ManagementRejection | undefined;
		const call = this.call(entry.id, { lane, idempotent: true });
		const tracked: ManagementCall = async (command, operationId) => {
			const response = await call(command, operationId);
			rejection = managementRejection(response) ?? rejection;
			return response;
		};
		const previous = entry.inspection;
		try {
			const value = await this.readInspection(tracked, entry.id, {
				expectedPlacements: previous?.value.placements.length,
				now: () => Math.round(this.ports.clock.now()),
				onPage: (pages) => {
					if (entry.generation !== generation) return;
					if (entry.inspection)
						entry.inspection = { ...entry.inspection, progress: { pages } };
					this.notify();
				},
			});
			if (entry.generation !== generation) return;
			const inspection: LiveInspection = {
				value,
				readAt: Math.floor(this.ports.clock.now() / 1000),
			};
			entry.inspection = inspection;
			this.markStep(entry, "reading_services", "done");
			this.retain(entry, value);
			this.options.onInspection?.(entry.id, inspection);
		} catch (error) {
			if (entry.generation !== generation) return;
			this.inspectionFailed(entry, error, rejection);
		}
	}

	private inspectionFailed(
		entry: Entry,
		error: unknown,
		rejection: ManagementRejection | undefined,
	): void {
		if (error instanceof LiveCallError && error.code === "keys_locked") return;
		const cause: LiveError = rejection
			? { step: "reading_services", code: "rejected", rejection }
			: (toLiveError(error) ?? { step: "session", code: "closed" });
		const slots = rejection?.code === "limit";
		this.markStep(
			entry,
			"reading_services",
			slots ? "skipped" : "failed",
			slots ? "slots_in_use" : stepDetail(liveErrorCode(cause)),
		);
		if (entry.inspection) {
			const { progress: _, ...read } = entry.inspection;
			entry.inspection = { ...read, error: cause };
		}
		this.notify();
		if (slots) {
			this.closeConnection(entry);
			this.scheduleRetry(entry, cause);
		}
	}

	/* Keys: a lock ends the session and clears live data. */

	private keysChanged(): void {
		for (const entry of this.entries.values()) {
			const controller = this.ports.keys.controller(entry.id);
			const unlocked = controller !== undefined && controller !== entry.keys;
			entry.keys = controller;
			if (entry.controller && controller !== entry.controller) {
				entry.controller = undefined;
				this.lockEntry(entry);
			} else if (!controller && entry.connecting) this.lockEntry(entry);
			if (unlocked) this.serve(entry);
		}
	}

	/** Demand taken while the device was locked is served once its keys open; an attempt the lock ended settles first. */
	private serve(entry: Entry): void {
		const ended = entry.connecting;
		if (!ended) {
			this.ensure(entry);
			return;
		}
		const settled = () => this.ensure(entry);
		ended.then(settled, settled);
	}

	private lockEntry(entry: Entry): void {
		this.shutdown(
			entry,
			new LiveCallError("keys_locked", "The device was locked."),
		);
		entry.inspection = undefined;
		entry.writer = undefined;
		entry.writerController = undefined;
		entry.steps = freshSteps();
		this.notify();
	}

	private shutdown(entry: Entry, error: LiveCallError): void {
		entry.generation++;
		entry.inspectAgain = false;
		entry.abort?.abort();
		entry.dataTunnel?.close();
		entry.dataTunnel = undefined;
		this.closeConnection(entry);
		for (const name of Object.keys(entry.timers) as TimerName[])
			if (name !== "inspect") this.cancel(entry, name);
		this.rejectQueued(entry, error);
		entry.attempt = 0;
		this.setState(entry, { kind: "idle" });
	}
}

export function createLiveSessionManager(
	deps: WorkspaceDeps,
	ports: LivePorts,
	options: LiveManagerOptions = {},
): LiveSessionManagerImpl {
	return new LiveManager(deps, ports, options);
}
