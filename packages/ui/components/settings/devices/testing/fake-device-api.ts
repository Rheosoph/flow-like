import { sha256 } from "@noble/hashes/sha2";
import { ApiResponseError } from "../../../../lib/api-error";
import type { AcmeCertificate } from "../../../../lib/device-management/certificate-acme";
import type {
	CertificateIssuer,
	CertificateRequest,
} from "../../../../lib/device-management/certificate-issuance";
import type {
	CertificateInventory,
	PublicCertificateInventory,
} from "../../../../lib/device-management/certificates";
import {
	base64url,
	unbase64url,
} from "../../../../lib/device-management/crypto";
import {
	type DeploymentRolloutStatus,
	type DeploymentVariable,
	eventEligibility,
} from "../../../../lib/device-management/deployment";
import type {
	BillingEligibility,
	BillingUsage,
	CertificateNotice,
} from "../../../../lib/device-management/hub/endpoints";
import {
	APPS,
	CRM_PLAN_APP,
	VISITOR_PLAN_APP,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import { sampleFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { AppInput } from "../../../../lib/device-management/model/app-plan";
import { presence } from "../../../../lib/device-management/model/presence";
import type {
	AccountBackupList,
	AgentFeature,
	AgentFeatures,
	AppDevicePlacements,
	ArchiveUsage,
	DeviceIdentity,
	DeviceLimits,
	DeviceRow,
	DeviceUsage,
	HubEnrollment,
	InspectionPlus,
	MyAccess,
	PlacementStatusPlus,
	ResourceSummary,
} from "../../../../lib/device-management/model/types";
import type { OfflineQueueStatus } from "../../../../lib/device-management/offline-queue";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import type {
	DeviceAccountScope,
	LocalDeviceVault,
} from "../../../../lib/device-management/storage";
import type {
	AccountRecoveryContext,
	ArchiveRoster,
	ControllerPublic,
	DeviceReceipt,
	Ed25519PublicKey,
	EncryptedFleetSnapshot,
	EncryptedInventory,
	FleetAudience,
	FleetReaderState,
	FleetView,
	InventoryScope,
	InventoryView,
	ManagementPolicy,
	OnboardingManifest,
	PolicyView,
	SignalingAdmission,
	TelemetryRoster,
} from "../../../../lib/device-management/types";
import type {
	BillingGrant,
	DeviceResources,
	ResourceGrant,
} from "../../../../lib/device-resources";
import type { IApiState } from "../../../../state/backend-state/api-state";
import type { IProfile } from "../../../../types";

/*
 * In-memory hub and device agents for device tests (plan §2.4 "Testing support").
 *
 *   const api = fakeDeviceApi();                         // golden sample fleet, current hub
 *   const old = fakeDeviceApi({ hubVersion: "old" });    // 404/405 on every route this work adds
 *   const legacy = fakeDeviceApi({ agentFeatures: {} }); // agents from before the `features` map
 *
 * The fake signs and encrypts nothing: "signatures" are checksums and "ciphertext"
 * is encoded plaintext, in formats only the fake crypto of `fake-workspace.ts` reads.
 */

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const utf8 = (text: string) => encoder.encode(text);
const DAY = 86_400;
const RELAY_PROTOCOL = "flowlike.device-management.v1";
const MAX_AGENT_REPLY_BYTES = 16 * 1024;

export const FAKE_PASSWORD = "correct horse battery staple";

/* Fake encodings shared with the fake crypto. */

export function fakeBytes(label: string): Uint8Array {
	return sha256(utf8(label));
}

/** SHA-256 in base64url, the value `digestText` computes. */
export function fakeDigest(text: string): string {
	return base64url(fakeBytes(text));
}

export function fakeKey(label: string): Ed25519PublicKey {
	return { kty: "OKP", crv: "Ed25519", x: base64url(fakeBytes(label)) };
}

const COMPACT_HEADER = base64url(utf8('{"alg":"fake"}'));
const signatureOf = (body: string, signer?: Ed25519PublicKey) =>
	base64url(fakeBytes(`signature:${signer?.x ?? "hub"}:${body}`).slice(0, 12));

/** A compact "JWS" whose signature is a checksum over the payload and the signer's public key. */
export function fakeCompact(
	payload: unknown,
	signer?: Ed25519PublicKey,
): string {
	const body = base64url(utf8(JSON.stringify(payload)));
	return `${COMPACT_HEADER}.${body}.${signatureOf(body, signer)}`;
}

export function readFakeCompact<T>(
	compact: string,
	signer?: Ed25519PublicKey,
): T {
	const [, body, signature] = compact.split(".");
	if (!body || signature !== signatureOf(body, signer))
		throw new Error("The signature does not verify.");
	return JSON.parse(decoder.decode(unbase64url(body, 65_536))) as T;
}

export const fakeKeys = {
	identity: (deviceId: string): DeviceIdentity => ({
		auth_key: fakeKey(`auth:${deviceId}`),
		telemetry_key: fakeKey(`telemetry:${deviceId}`),
		management_key: Array.from(fakeBytes(`management:${deviceId}`)),
	}),
	controller: (userId: string, deviceId: string) =>
		fakeKey(`controller:${userId}:${deviceId}`),
	invitation: (ownerId: string, deviceId: string) =>
		fakeKey(`invitation:${ownerId}:${deviceId}`),
	bootstrap: (deviceId: string) => fakeKey(`bootstrap:${deviceId}`),
};

export function fakeControllerPublic(
	userId: string,
	deviceId: string,
	endpointId = "endpoint-1",
): ControllerPublic {
	return {
		device_id: deviceId,
		endpoint_id: endpointId,
		controller_key: fakeKeys.controller(userId, deviceId),
		archive_key: Array.from(fakeBytes(`archive:${userId}:${deviceId}`)),
		telemetry_member: {
			endpoint_id: endpointId,
			signing_key: fakeKey(`member:${userId}:${deviceId}:${endpointId}`),
		},
	};
}

/** What a fake vault holds instead of keys: a password check, never the password. */
export interface FakeVaultSecret {
	fake: "controller" | "invitation";
	device_id: string;
	check: string;
	controller?: ControllerPublic;
	public_key?: Ed25519PublicKey;
	/** Makes a re-sealed vault differ byte for byte, as a new salt does. */
	nonce?: string;
}

export function fakePasswordCheck(
	deviceId: string,
	password: Uint8Array,
): string {
	return fakeDigest(`password:${deviceId}:${decoder.decode(password)}`);
}

export function sealFakeVault(secret: FakeVaultSecret): Uint8Array {
	return utf8(JSON.stringify(secret).padEnd(96, " "));
}

export function openFakeVault(bytes: Uint8Array): FakeVaultSecret {
	return JSON.parse(decoder.decode(bytes)) as FakeVaultSecret;
}

interface FakeRecovery {
	fake: "recovery";
	device_id: string;
	revision: number;
	check: string;
	backup: string;
}

export function sealFakeRecovery(
	context: AccountRecoveryContext,
	password: Uint8Array,
	backup: Uint8Array,
): { ciphertext: number[]; proof_jws: string } {
	const sealed: FakeRecovery = {
		fake: "recovery",
		device_id: context.device_id,
		revision: context.revision,
		check: fakePasswordCheck(context.device_id, password),
		backup: base64url(backup),
	};
	return {
		ciphertext: Array.from(utf8(JSON.stringify(sealed))),
		proof_jws: fakeCompact(
			{ device_id: context.device_id, revision: context.revision },
			context.controller_key,
		),
	};
}

export function openFakeRecovery(
	context: AccountRecoveryContext,
	password: Uint8Array,
	ciphertext: Uint8Array,
): unknown {
	const sealed = JSON.parse(decoder.decode(ciphertext)) as FakeRecovery;
	if (
		sealed.fake !== "recovery" ||
		sealed.device_id !== context.device_id ||
		sealed.revision !== context.revision ||
		sealed.check !== fakePasswordCheck(context.device_id, password)
	)
		throw new Error("The backup cannot be opened with this password.");
	return JSON.parse(decoder.decode(unbase64url(sealed.backup, 1024 * 1024)));
}

/** The plaintext envelope `encryptedControllerBackup` writes, as account backups seal it. */
function backupEnvelope(
	apiOrigin: string,
	vault: LocalDeviceVault,
): Uint8Array {
	return utf8(
		JSON.stringify({
			version: 1,
			apiOrigin,
			deviceId: vault.deviceId,
			controllerPublic: vault.controllerPublic,
			controllerVault: Array.from(vault.controllerVault),
			invitationVault: vault.invitationVault
				? Array.from(vault.invitationVault)
				: undefined,
			manifestJws: vault.manifestJws,
			grantId: vault.grantId,
			ownerControllerKey: vault.ownerControllerKey,
		}),
	);
}

/* Public surface. */

export type DeviceSeed = ReturnType<typeof sampleFleet>;
export type HubVersion = "current" | "old";
export type FakeCall = [method: string, path: string, body?: unknown];
export type FakeCommand = [
	deviceId: string,
	type: string,
	command: Record<string, unknown>,
];

export interface FakeDeviceApiOptions {
	seed?: DeviceSeed;
	/** `"old"` answers every route this work adds like a hub from before it: 404 or 405. */
	hubVersion?: HubVersion;
	/** One feature set for every agent, or one per device id. `{}` is an agent from before the `features` map. */
	agentFeatures?: AgentFeatures | Record<string, AgentFeatures>;
	origin?: string;
	issuer?: string;
	profileId?: string;
	/** The password every seeded vault and account backup opens with. */
	password?: string;
	/** Milliseconds; defaults to the seed's `now`. */
	now?: () => number;
}

export type HubMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export interface HubRequest {
	params: Record<string, string>;
	query: URLSearchParams;
	body: unknown;
}

export interface RouteMatch {
	method?: HubMethod;
	/** The path without its query (`devices/usage`), or a pattern tested against path and query. */
	path: string | RegExp;
}

export interface CommandContext {
	agent: FakeAgent;
	operationId: string;
}

export interface CommandReply {
	state: string;
	result: Record<string, unknown>;
}

export type CommandHandler = (
	command: Record<string, unknown>,
	context: CommandContext,
) => CommandReply | Promise<CommandReply>;

export interface TelemetryRow {
	sequence: number;
	timestamp: number;
	kind: string;
	data: Record<string, unknown>;
}

export interface FakeDeviceApi extends IApiState {
	readonly seed: DeviceSeed;
	readonly hub: FakeHub;
	/** The signed-in account's storage scope and a profile pointing at this hub. */
	readonly scope: DeviceAccountScope;
	readonly profile: IProfile;
	/** Every hub request in order, with secret-bearing fields redacted. */
	readonly calls: FakeCall[];
	/** Every management command an agent received, with secret values redacted. */
	readonly commands: FakeCommand[];
	/** Whole-hub conditions for the area gates. */
	readonly mode: HubMode;
	agent(deviceId: string): FakeAgent;
	/** Add or replace a route; returns a function that removes it. */
	on(
		method: HubMethod,
		template: string,
		handle: (request: HubRequest) => unknown,
	): () => void;
	/** Fail matching requests (default: 503), `times` times or until the returned function is called. */
	fail(match: RouteMatch, error?: unknown, times?: number): () => void;
	/** Keep matching requests pending until the returned function is called. */
	hold(match: RouteMatch): () => void;
	sent(method?: HubMethod, path?: string | RegExp): FakeCall[];
	/** Hub requests that change something: everything but GET and live-session admissions. */
	writes(): FakeCall[];
	/** `fetch` for the unauthenticated hub JSON (`GET <origin>/api/v1`). */
	readonly hubFetch: typeof fetch;
	/** The signaling relay: install as `globalThis.WebSocket` for live sessions. */
	readonly WebSocket: typeof WebSocket;
}

export interface HubMode {
	/** False: every request fails like a dropped connection. */
	reachable: boolean;
	/** False: every request answers 401. */
	signedIn: boolean;
	/** True: device routes refuse the token (restricted personal access token). */
	tokenRestricted: boolean;
	/** False: the hub JSON says devices are off and device routes answer 503. */
	devicesEnabled: boolean;
	/** False: live-session admissions answer 503 (signaling not configured). */
	signaling: boolean;
}

/* Errors as the hub sends them. */

const hubError = (status: number, code: string | undefined, message: string) =>
	new ApiResponseError({ status, code, message });
const notFound = (what: string) =>
	hubError(404, "NOT_FOUND", `${what} was not found`);
const badRequest = (message: string) => hubError(400, "BAD_REQUEST", message);
const forbidden = (message: string) => hubError(403, "FORBIDDEN", message);
const conflict = (message: string) => hubError(409, "CONFLICT", message);

/* Redaction: neither `calls` nor `commands` may hold a secret. */

const REDACTED = "[redacted]";
const SECRET_FIELDS = new Set([
	"password",
	"private_key_pem",
	"ciphertext",
	"proof_jws",
	"enrollment_token",
	"bootstrap_secret",
	"secret_base64",
	"release_jws",
]);
const SECRET_VALUE_COMMANDS = new Set(["set_secret", "rollout_secret"]);

function redact(value: unknown, secretValue = false): unknown {
	if (Array.isArray(value)) return value.map((item) => redact(item));
	if (!value || typeof value !== "object") return value;
	return Object.fromEntries(
		Object.entries(value).map(([key, item]) => [
			key,
			SECRET_FIELDS.has(key) || (secretValue && key === "value")
				? REDACTED
				: redact(item),
		]),
	);
}

/* Agent wire shapes. */

const PLACEMENT_BASE = [
	"id",
	"project_id",
	"deployment_id",
	"revision",
	"desired_state",
	"observed_state",
	"config_revision",
	"intent_revision",
	"applied_revision",
] as const;
const DIAGNOSTIC_FIELDS = [
	"process_id",
	"last_error",
	"has_error",
	"restarts",
] as const;
const EVENT_FIELDS = [
	"source",
	"events",
	"events_truncated",
	"online_metadata_sha256",
] as const;
const SNAPSHOT_FIELDS = ["has_error", ...EVENT_FIELDS] as const;

function pick<T extends object>(
	row: T,
	keys: readonly (keyof T)[],
): Partial<T> {
	const output: Partial<T> = {};
	for (const key of keys) if (row[key] !== undefined) output[key] = row[key];
	return output;
}

function replicasOf(row: PlacementStatusPlus) {
	if (row.replicas) return row.replicas;
	const running = row.running_replicas ?? 0;
	return Array.from({ length: row.desired_replicas ?? 1 }, (_, slot) => ({
		slot,
		observed_state: slot < running ? "running" : row.observed_state,
		applied_revision: row.applied_revision,
	}));
}

const COUNT_DEFAULTS = {
	desired_replicas: 1,
	running_replicas: 0,
	ready_replicas: 0,
	max_replicas: 1,
};

function placementCounts(row: PlacementStatusPlus) {
	return {
		...COUNT_DEFAULTS,
		...pick(row, [
			"desired_replicas",
			"running_replicas",
			"ready_replicas",
			"max_replicas",
		]),
	};
}

/** A placement row as `inspect_page` sends it; fields behind a missing feature flag are left out. */
function wirePlacement(
	row: PlacementStatusPlus,
	features: AgentFeatures,
): Record<string, unknown> {
	const diagnostics = features.placement_diagnostics === 1;
	return {
		...pick(row, PLACEMENT_BASE),
		...placementCounts(row),
		replicas: replicasOf(row).map((replica) =>
			diagnostics
				? replica
				: pick(replica, ["slot", "observed_state", "applied_revision"]),
		),
		...(diagnostics ? pick(row, DIAGNOSTIC_FIELDS) : {}),
		...(features.placement_events === 1 ? pick(row, EVENT_FIELDS) : {}),
		...(features.offline_summary === 1 ? pick(row, ["offline_writes"]) : {}),
	};
}

/** A placement row of an encrypted status snapshot: never error text, process ids or retry times. */
function snapshotPlacement(row: PlacementStatusPlus): Record<string, unknown> {
	return {
		...pick(row, PLACEMENT_BASE),
		...placementCounts(row),
		replicas: replicasOf(row).map((replica) =>
			pick(replica, ["slot", "observed_state", "applied_revision"]),
		),
		...pick(row, SNAPSHOT_FIELDS),
	};
}

const ALL_FEATURES: AgentFeatures = {
	placement_diagnostics: 1,
	task_health: 1,
	placement_events: 1,
	offline_summary: 1,
	host_operation: 1,
	network_interfaces: 1,
	rollout_history: 1,
	operations: 1,
	metrics_history: 1,
	offline_lookup: 1,
	reader_bindings: 1,
	acme_failure_detail: 1,
	archive_status: 1,
	artifact_capacity: 1,
};

/** The feature flag each command of plan §3.4.3 needs. */
const COMMAND_FEATURE: Record<string, AgentFeature> = {
	host_operation: "host_operation",
	rollout_history: "rollout_history",
	operations: "operations",
	metrics_history: "metrics_history",
	offline_queue_operations: "offline_lookup",
	offline_queue_lookup: "offline_lookup",
};
const ARTIFACT_FEATURE_KINDS = new Set(["usage", "prune"]);

const completed = (result: Record<string, unknown> = {}): CommandReply => ({
	state: "completed",
	result,
});

/** A coded refusal, as agents send it. */
export function rejected(
	code: string,
	error: string,
	retryable = false,
): CommandReply {
	return { state: "rejected", result: { code, error, retryable } };
}

const unsupported = () => rejected("unsupported", "Unsupported command.");

function pageOf<T>(
	rows: readonly T[],
	keyOf: (row: T) => string,
	after: unknown,
	limit: number,
): { rows: T[]; next: string | null } {
	const sorted = [...rows].sort((a, b) => (keyOf(a) < keyOf(b) ? -1 : 1));
	const start =
		typeof after === "string"
			? sorted.findIndex((row) => keyOf(row) > after)
			: 0;
	const page = start < 0 ? [] : sorted.slice(start, start + limit);
	const more = start >= 0 && start + limit < sorted.length;
	const last = page.at(-1);
	return { rows: page, next: more && last ? keyOf(last) : null };
}

const limitOf = (command: Record<string, unknown>, fallback: number) =>
	typeof command.limit === "number" && command.limit >= 1
		? Math.floor(command.limit)
		: fallback;

const text = (value: unknown) => (typeof value === "string" ? value : "");

/* The agent. */

/** An upload the device holds, in the shape its `status` answer has. */
export interface FakeTransfer {
	transfer_id: string;
	descriptor: Record<string, unknown>;
	state: "receiving" | "committed" | "aborted";
	expires_at: number;
	manifest_ready: boolean;
	file_index: number | null;
	offset: number;
	complete: boolean;
	project_path: string | null;
}

interface AgentSeed {
	inspection?: InspectionPlus;
	placements: PlacementStatusPlus[];
	bootId: string;
	live?: DeviceSeed["live"][string];
	transfers: FakeTransfer[];
}

type RosterKind = "telemetry" | "logs" | "metrics";

const READ_FACT_KEYS = [
	"placements",
	"rollouts",
	"certificates",
	"certificateRequests",
	"certificateIssuers",
	"acme",
	"offlineQueues",
] as const;

/** What the seed says the device holds beyond its inspection; nothing where the seed is silent. */
function readFacts(live: AgentSeed["live"]) {
	const empty: Required<
		Pick<NonNullable<AgentSeed["live"]>, (typeof READ_FACT_KEYS)[number]>
	> = {
		placements: {},
		rollouts: [],
		certificates: { inventory_revision: 0, certificates: [] },
		certificateRequests: [],
		certificateIssuers: [],
		acme: [],
		offlineQueues: {},
	};
	return structuredClone({
		...empty,
		...(live ? pick(live, READ_FACT_KEYS) : {}),
	});
}

export class FakeAgent {
	/** `undefined` follows the hub row: reachable while its last check-in is at most 10 minutes old. */
	online?: boolean;
	features: AgentFeatures;
	bootId: string;
	/** Device-scope facts of inspection page 0, in wire form. */
	facts: Record<string, unknown>;
	placements: PlacementStatusPlus[];
	/** Placement configurations by placement id; derived from the placement row when absent. */
	configs = new Map<string, Record<string, unknown>>();
	/** Seeded endpoint, TLS, offline-write and cloud-approval facts by placement id. */
	configFacts: NonNullable<DeviceSeed["live"][string]["placements"]>;
	rollouts: DeploymentRolloutStatus[];
	certificates: CertificateInventory;
	certificateRequests: CertificateRequest[];
	certificateIssuers: CertificateIssuer[];
	acme: AcmeCertificate[];
	offlineQueues: Record<string, OfflineQueueStatus[]>;
	/** Signed rosters by `<telemetry|logs|metrics>:<scope>`, with the fields the agent attaches. */
	rosters = new Map<string, { text: string; extra: Record<string, unknown> }>();
	/** Telemetry by `<metrics|logs|messages>:<placement id or "device" or "project:<id>">`. */
	records = new Map<string, TelemetryRow[]>();
	/** Answers of commands sent with an operation id (the device's 24 h journal). */
	journal = new Map<string, CommandReply & { type: string; at: number }>();
	/** Configurations of staged updates by rollout id, applied on activation. */
	staged = new Map<string, Record<string, unknown>>();
	/** Activated rollouts that turn healthy on their next status read. */
	settling = new Set<string>();
	/** Uploads the device still holds, by transfer id. */
	transfers = new Map<string, FakeTransfer>();
	private readonly handlers = new Map<string, CommandHandler>();
	private readonly drops: (string | undefined)[] = [];
	private readonly sockets = new Set<{ remoteClose(): void }>();

	constructor(
		readonly deviceId: string,
		private readonly world: World,
		seed: AgentSeed,
		features: AgentFeatures | undefined,
	) {
		const inspection = seed.inspection;
		this.features = features ?? inspection?.features ?? ALL_FEATURES;
		this.bootId = seed.bootId;
		this.placements = structuredClone(seed.placements);
		this.facts = inspection ? inspectionFacts(inspection) : {};
		const read = readFacts(seed.live);
		this.configFacts = read.placements;
		this.rollouts = read.rollouts;
		this.certificates = read.certificates;
		this.certificateRequests = read.certificateRequests;
		this.certificateIssuers = read.certificateIssuers;
		this.acme = read.acme;
		this.offlineQueues = read.offlineQueues;
		for (const transfer of seed.transfers)
			this.transfers.set(transfer.transfer_id, transfer);
		this.seedRosters(seed.live);
	}

	private seedRosters(live: AgentSeed["live"]) {
		const owner = this.world.hub.ownerInvitationKey(this.deviceId);
		for (const entry of live?.history ?? []) {
			const roster: ArchiveRoster = {
				version: 1,
				device_id: this.deviceId,
				scope: entry.scope,
				project_id: null,
				kind: entry.kind,
				policy_version: entry.policyVersion,
				previous_policy_digest: null,
				management_policy_digest: null,
				recipients: [],
				issued_at: entry.expiresAt - 30 * DAY,
				expires_at: entry.expiresAt,
			};
			this.setRoster(entry.kind, entry.scope, fakeCompact(roster, owner), {
				...(entry.status ? { status: entry.status } : {}),
			});
		}
		for (const entry of live?.metricReaders ?? []) {
			const roster: TelemetryRoster = {
				version: 1,
				device_id: this.deviceId,
				scope: entry.scope,
				policy_version: 1,
				previous_policy_digest: null,
				management_policy_digest: null,
				publisher: {
					endpoint_id: this.deviceId,
					signing_key: fakeKeys.identity(this.deviceId).telemetry_key,
				},
				members: [],
				issued_at: entry.expiresAt - 30 * DAY,
				expires_at: entry.expiresAt,
			};
			this.setRoster("telemetry", entry.scope, fakeCompact(roster, owner), {
				confirmed_readers: [],
			});
		}
	}

	setRoster(
		kind: RosterKind,
		scope: string,
		signed: string,
		extra: Record<string, unknown> = {},
	) {
		this.rosters.set(`${kind}:${scope}`, { text: signed, extra });
	}

	/** Unix seconds on the hub's clock. */
	now(): number {
		return this.world.hub.now();
	}

	get reachable(): boolean {
		if (this.online !== undefined) return this.online;
		const row = this.world.hub.rows.get(this.deviceId);
		if (!row) return false;
		const kind = presence(row, this.now()).kind;
		return kind === "online" || kind === "late";
	}

	/** Open live sessions to this device. */
	get sessions(): number {
		return this.sockets.size;
	}

	/** Answer `type` with `handler` instead of the default; returns a function that restores the default. */
	handle(type: string, handler: CommandHandler): () => void {
		this.handlers.set(type, handler);
		return () => {
			if (this.handlers.get(type) === handler) this.handlers.delete(type);
		};
	}

	/** Refuse `type` with a coded rejection until the returned function is called. */
	reject(
		type: string,
		code: string,
		error: string,
		retryable = false,
	): () => void {
		return this.handle(type, () => rejected(code, error, retryable));
	}

	/** Keep answers to `type` pending until the returned function is called. */
	hold(type: string): () => void {
		let release!: () => void;
		const gate = new Promise<void>((resolve) => {
			release = resolve;
		});
		const previous = this.handlers.get(type);
		const restore = this.handle(type, async (command, context) => {
			await gate;
			return (previous ?? defaultHandler(type))(command, context);
		});
		return () => {
			restore();
			if (previous) this.handlers.set(type, previous);
			release();
		};
	}

	/** The next command (of `type`, or any) gets no answer: the session drops, as on a lost connection. */
	dropNext(type?: string) {
		this.drops.push(type);
	}

	/** Drop every open session from the device side. */
	disconnect() {
		for (const socket of [...this.sockets]) socket.remoteClose();
	}

	/** Reboot: a new boot id and no open sessions. */
	reboot(bootId = crypto.randomUUID().replaceAll("-", "").slice(0, 16)) {
		this.bootId = bootId;
		this.disconnect();
	}

	record(key: string, data: Record<string, unknown>, kind = "sample") {
		const rows = this.records.get(key) ?? [];
		rows.push({
			sequence: (rows.at(-1)?.sequence ?? 0) + 1,
			timestamp: this.now(),
			kind,
			data,
		});
		this.records.set(key, rows);
	}

	placement(placementId: unknown): PlacementStatusPlus | undefined {
		return this.placements.find((row) => row.id === placementId);
	}

	/** @internal The relay hands every decrypted request here. */
	async receive(
		socket: { remoteClose(): void },
		request: { operation_id: string; command: Record<string, unknown> },
	): Promise<Record<string, unknown> | undefined> {
		const { command, operation_id: operationId } = request;
		const type = text(command.type);
		this.world.commands.push([
			this.deviceId,
			type,
			redact(command, SECRET_VALUE_COMMANDS.has(type)) as Record<
				string,
				unknown
			>,
		]);
		const drop = this.drops.findIndex(
			(wanted) => wanted === undefined || wanted === type,
		);
		if (drop >= 0) {
			this.drops.splice(drop, 1);
			socket.remoteClose();
			return undefined;
		}
		const reply = await this.answer(type, command, operationId);
		if (type !== "operation" && !READS.has(type))
			this.journal.set(operationId, {
				...reply,
				type,
				at: this.now(),
			});
		const lookup =
			type === "operation" && reply.state !== "rejected"
				? text(command.operation_id)
				: operationId;
		return this.bounded({ operation_id: lookup, ...reply }, operationId);
	}

	private bounded(reply: Record<string, unknown>, operationId: string) {
		if (utf8(JSON.stringify(reply)).length <= MAX_AGENT_REPLY_BYTES)
			return reply;
		return {
			operation_id: operationId,
			...rejected("limit", "The answer exceeds one management message."),
		};
	}

	private async answer(
		type: string,
		command: Record<string, unknown>,
		operationId: string,
	): Promise<CommandReply> {
		const context: CommandContext = { agent: this, operationId };
		const custom = this.handlers.get(type);
		if (custom) return custom(command, context);
		if (!this.supports(type, command)) return unsupported();
		return defaultHandler(type)(command, context);
	}

	private supports(type: string, command: Record<string, unknown>) {
		const feature =
			type === "artifact"
				? artifactFeature(command)
				: (COMMAND_FEATURE[type] as AgentFeature | undefined);
		return !feature || this.features[feature] === 1;
	}

	/** @internal */
	attach(socket: { remoteClose(): void }) {
		this.sockets.add(socket);
	}

	/** @internal */
	detach(socket: { remoteClose(): void }) {
		this.sockets.delete(socket);
	}

	/** @internal Inspection rows in the id order agents page them in. */
	rows(): Record<string, unknown>[] {
		return [...this.placements]
			.sort((a, b) => (a.id < b.id ? -1 : 1))
			.map((row) => wirePlacement(row, this.features));
	}

	/** @internal Page-0 facts; an agent without feature flags sends none of the newer ones. */
	pageFacts(): Record<string, unknown> {
		const flags = this.features;
		const legacy = Object.keys(flags).length === 0;
		const hidden = new Set<string>([
			...(legacy ? ["agent", "host"] : []),
			...(flags.task_health === 1 ? [] : ["tasks"]),
			...(flags.host_operation === 1 ? [] : ["host_operation"]),
			...(flags.network_interfaces === 1 ? [] : ["network"]),
		]);
		return {
			...Object.fromEntries(
				Object.entries(this.facts).filter(([key]) => !hidden.has(key)),
			),
			...(legacy ? {} : { features: flags }),
		};
	}
}

function artifactFeature(
	command: Record<string, unknown>,
): AgentFeature | undefined {
	const kind = (command.request as { kind?: unknown } | undefined)?.kind;
	return typeof kind === "string" && ARTIFACT_FEATURE_KINDS.has(kind)
		? "artifact_capacity"
		: undefined;
}

function inspectionFacts(value: InspectionPlus): Record<string, unknown> {
	const facts: Record<string, unknown> = {
		certificate_management: value.certificate_management,
		can_manage_certificates: value.can_manage_certificates,
		certificate_issuance: value.certificate_issuance,
		certificate_acme: value.certificate_acme,
		can_delegate_certificate_renewal: value.can_delegate_certificate_renewal,
		agent_version: value.agentVersion,
		host_operations: value.hostOperations,
		host_isolation: value.hostIsolation,
		isolation: value.isolation,
		agent: value.agent,
		host: value.host,
		tasks: value.tasks,
		host_operation: value.hostOperation,
		network: value.network,
	};
	return Object.fromEntries(
		Object.entries(facts).filter(([, fact]) => fact !== undefined),
	);
}

/* Default command handlers. */

const READS = new Set([
	"inspect_page",
	"operation",
	"placement_configuration",
	"rollout",
	"metrics",
	"project_metrics",
	"logs",
	"messages",
	"telemetry_read",
	"telemetry_roster_read",
	"archive_roster_read",
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

const revisionConflict = (placementId: unknown) =>
	rejected(
		"revision_conflict",
		`Placement ${text(placementId)} changed on the device.`,
	);

/** The placement a command names, or the refusal when it is unknown or its revision moved. */
function target(
	agent: FakeAgent,
	command: Record<string, unknown>,
): PlacementStatusPlus | CommandReply {
	const row = agent.placement(command.placement_id);
	if (!row)
		return rejected(
			"invalid",
			`Unknown placement ${text(command.placement_id)}.`,
		);
	return command.expected_revision !== undefined &&
		command.expected_revision !== row.config_revision
		? revisionConflict(row.id)
		: row;
}

const isReply = (
	value: PlacementStatusPlus | CommandReply,
): value is CommandReply => "result" in value;

function setRunning(row: PlacementStatusPlus, running: boolean) {
	const count = running ? (row.desired_replicas ?? 1) : 0;
	row.desired_state = running ? "running" : "stopped";
	row.observed_state = running ? "running" : "stopped";
	row.running_replicas = count;
	row.ready_replicas = count;
	row.intent_revision += 1;
	row.replicas = undefined;
}

function lifecycle(running: boolean): CommandHandler {
	return (command, { agent }) => {
		const row = target(agent, command);
		if (isReply(row)) return row;
		setRunning(row, running);
		return completed({ placement_id: row.id, state: row.observed_state });
	};
}

const scale: CommandHandler = (command, { agent }) => {
	const row = target(agent, command);
	if (isReply(row)) return row;
	const replicas = Number(command.replicas);
	if (!Number.isInteger(replicas) || replicas < 1)
		return rejected("invalid", "Choose at least one instance.");
	if (replicas > (row.max_replicas ?? 1))
		return rejected("limit", "More instances than this service allows.");
	row.desired_replicas = replicas;
	if (row.desired_state === "running") setRunning(row, true);
	return completed({ placement_id: row.id, replicas });
};

const remove: CommandHandler = (command, { agent }) => {
	const row = target(agent, command);
	if (isReply(row)) return row;
	agent.placements = agent.placements.filter((other) => other !== row);
	return completed({ placement_id: row.id, removed: true });
};

function hostOperation(kind: "reboot" | "update_agent"): CommandHandler {
	return (command, { agent, operationId }) => {
		if (command.expected_boot_id !== agent.bootId)
			return rejected("revision_conflict", "The device has restarted.");
		const allowed = agent.facts.host_operations as
			| Record<string, boolean>
			| undefined;
		if (allowed?.[kind] !== true)
			return rejected("host_policy", "This device does not allow it.");
		agent.facts.host_operation = {
			operation_id: operationId,
			kind,
			state: "requested",
			created_at: agent.now(),
			issued_by: "you",
		};
		return { state: "accepted", result: { kind } };
	};
}

const inspectPage: CommandHandler = (command, { agent }) => {
	const { rows, next } = pageOf(
		agent.rows(),
		(row) => text(row.id),
		command.after,
		Math.min(limitOf(command, 2), 2),
	);
	return completed({
		device_id: agent.deviceId,
		boot_id: agent.bootId,
		placements: rows,
		next,
		...(command.after == null ? agent.pageFacts() : {}),
	});
};

function recordKey(prefix: string, command: Record<string, unknown>) {
	if (typeof command.placement_id === "string")
		return `${prefix}:${command.placement_id}`;
	return typeof command.project_id === "string"
		? `${prefix}:project:${command.project_id}`
		: `${prefix}:device`;
}

function telemetry(prefix: string): CommandHandler {
	return (command, { agent }) => {
		const after = typeof command.after === "number" ? command.after : 0;
		const rows = (agent.records.get(recordKey(prefix, command)) ?? [])
			.filter((row) => row.sequence > after)
			.slice(0, limitOf(command, 100));
		return completed({
			records: rows,
			next: rows.at(-1)?.sequence ?? after,
		});
	};
}

/** Agents answer live metrics with the newest samples of the scope. */
const metrics: CommandHandler = (command, { agent }) => {
	const rows = agent.records.get(recordKey("metrics", command)) ?? [];
	return completed({ records: rows.slice(-8) });
};

function activeRollout(agent: FakeAgent, placementId: string) {
	return (
		agent.rollouts.find(
			(row) =>
				row.placement_id === placementId &&
				["staged", "validating", "activating", "rolling_back"].includes(
					row.state,
				),
		) ?? null
	);
}

type ConfigFacts = FakeAgent["configFacts"][string];

function configEvents(row: PlacementStatusPlus) {
	if (row.events?.length) return row.events;
	return [
		{
			event_id: `${row.id}-event`,
			event_version: [1, 0, 0],
			board_version: [1, 0, 0],
		},
	];
}

function configHosting(row: PlacementStatusPlus, facts: ConfigFacts) {
	if (!facts?.port) return null;
	return {
		host: facts.host ?? "127.0.0.1",
		port: facts.port,
		max_in_flight: 32,
		request_timeout_secs: 60,
		auth_secret: `${row.id}-auth`,
	};
}

function configOfflineWrites(facts: ConfigFacts) {
	const writes = facts?.offlineWrites;
	if (!writes) return null;
	return {
		tables: [
			{
				purpose: "storage",
				database: "db",
				table: "records",
				primary_key: "id",
			},
		],
		files: [],
		max_queue_bytes: writes.maxBytes,
		max_operations: 10_000,
		max_age_seconds: writes.maxAgeS,
		max_mirror_bytes: 2 * 1024 ** 3,
	};
}

/** Only an online service has a cloud approval and may buffer writes. */
function configOnline(row: PlacementStatusPlus, facts: ConfigFacts) {
	if (row.source !== "online")
		return { resource_grant: null, offline_writes: null };
	return {
		resource_grant: {
			grant_id: facts?.resourceGrantId ?? `${row.id}-approval`,
			authz_version: 1,
		},
		offline_writes: configOfflineWrites(facts),
	};
}

/** A configuration that matches the placement row and the seed's endpoint facts. */
function derivedConfig(agent: FakeAgent, row: PlacementStatusPlus) {
	const facts = agent.configFacts[row.id];
	return {
		source: "offline",
		online_metadata_sha256: null,
		...pick(row, [
			"id",
			"project_id",
			"deployment_id",
			"revision",
			"source",
			"online_metadata_sha256",
		]),
		project_path: `projects/${row.project_id}`,
		events: configEvents(row),
		hosting: configHosting(row, facts),
		tls_certificate_id: facts?.tlsCertificateId ?? null,
		max_replicas: placementCounts(row).max_replicas,
		variables: {},
		secret_overrides: {},
		...configOnline(row, facts),
		resources: null,
	};
}

const placementConfiguration: CommandHandler = (command, { agent }) => {
	const row = agent.placement(command.placement_id);
	if (!row) return rejected("unauthorized", "No access to this placement.");
	return completed({
		placement_id: row.id,
		project_id: row.project_id,
		deployment_id: row.deployment_id,
		config_revision: row.config_revision,
		config: agent.configs.get(row.id) ?? derivedConfig(agent, row),
		desired_state: row.desired_state === "running" ? "running" : "stopped",
		rollout_sources: ["offline", "online"],
		rollout: activeRollout(agent, row.id),
	});
};

const ACTIVATING = new Set(["validating", "activating"]);

/** A reading client sees an activated rollout finish on its next status read. */
const rollout: CommandHandler = (command, { agent }) => {
	const row = agent.rollouts.find(
		(candidate) => candidate.rollout_id === command.rollout_id,
	);
	if (!row) return rejected("invalid", "Unknown rollout.");
	const reply = completed({ ...row });
	if (ACTIVATING.has(row.state) && agent.settling.delete(row.rollout_id)) {
		row.state = "healthy";
		row.updated_at = agent.now();
		const placement = agent.placement(row.placement_id);
		if (placement) setRunning(placement, true);
	}
	return reply;
};

const operation: CommandHandler = (command, { agent }) => {
	const known = agent.journal.get(text(command.operation_id));
	return known
		? { state: known.state, result: known.result }
		: rejected("invalid", "The device has no record of this operation.");
};

const certificates: CommandHandler = (command, { agent }) => {
	const { rows, next } = pageOf(
		agent.certificates.certificates,
		(row) => row.certificate_id,
		command.after,
		Math.min(limitOf(command, 4), 4),
	);
	return completed({
		certificates: rows,
		inventory_revision: agent.certificates.inventory_revision,
		next,
	});
};

function listed<T>(
	field: string,
	read: (agent: FakeAgent) => readonly T[],
	keyOf: (row: T) => string,
	max: number,
): CommandHandler {
	return (command, { agent }) => {
		const { rows, next } = pageOf(
			read(agent),
			keyOf,
			command.after,
			Math.min(limitOf(command, max), max),
		);
		return completed({ [field]: rows, next });
	};
}

const offlineQueue: CommandHandler = (command, { agent }) => {
	const placementId = text(command.placement_id);
	const { rows, next } = pageOf(
		agent.offlineQueues[placementId] ?? [],
		(row) => row.scope,
		command.after,
		2,
	);
	return completed({ placement_id: placementId, queues: rows, next });
};

/** One chunk of a signed roster, in the transfer format of `readTelemetryChunks`. */
function chunk(
	signed: string,
	command: Record<string, unknown>,
	extra: Record<string, unknown>,
): CommandReply {
	const bytes = utf8(signed);
	const offset = typeof command.offset === "number" ? command.offset : 0;
	const part = bytes.slice(offset, offset + limitOf(command, 4096));
	return completed({
		available: true,
		offset,
		total: bytes.length,
		digest: fakeDigest(signed),
		chunk: base64url(part),
		sequence: 1,
		latest: 1,
		...(offset === 0 ? extra : {}),
	});
}

function rosterRead(kindOf: (command: Record<string, unknown>) => string) {
	const handler: CommandHandler = (command, { agent }) => {
		const roster = agent.rosters.get(
			`${kindOf(command)}:${text(command.scope)}`,
		);
		return roster
			? chunk(roster.text, command, roster.extra)
			: completed({ available: false, latest: 0 });
	};
	return handler;
}

const rolloutHistory: CommandHandler = (command, { agent }) => {
	const placementId = text(command.placement_id);
	const rows = agent.rollouts
		.filter((row) => row.placement_id === placementId)
		.sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0))
		.slice(0, limitOf(command, 8));
	return completed({ placement_id: placementId, rollouts: rows, next: null });
};

const operations: CommandHandler = (command, { agent }) => {
	const rows = [...agent.journal.entries()]
		.slice(-limitOf(command, 20))
		.map(([operationId, entry]) => ({
			operation_id: operationId,
			kind: null,
			actor: { role: "owner", user_id: null, grant_id: null },
			project_id: null,
			placement_id: null,
			accepted_at: entry.at,
			state: entry.state === "rejected" ? "failed" : entry.state,
		}));
	return completed({ operations: rows, next: null });
};

const metricsHistory: CommandHandler = (command, { agent }) => {
	const fields = Array.isArray(command.fields)
		? command.fields.map(String)
		: [];
	const after = typeof command.after === "number" ? command.after : 0;
	const rows = (agent.records.get(recordKey("metrics", command)) ?? [])
		.filter((row) => row.sequence > after)
		.slice(0, limitOf(command, 256));
	const value = (row: TelemetryRow, field: string) =>
		typeof row.data[field] === "number" ? row.data[field] : null;
	return completed({
		fields,
		points: rows.map((row) => [
			row.timestamp,
			...fields.map((field) => value(row, field)),
		]),
		next: null,
		evicted_through: null,
	});
};

const EMPTY_BUDGET = {
	bytes: { used: 0, max: null },
	entries: { used: 0, max: null },
	revisions: { used: 0, max: null },
};

type ArtifactHandler = (
	request: Record<string, unknown>,
	agent: FakeAgent,
) => CommandReply;

/** An unknown upload is a coded `failed`, as on the device. */
function transferOf(agent: FakeAgent, request: Record<string, unknown>) {
	return (
		agent.transfers.get(text(request.transfer_id)) ??
		rejected("failed", "The device does not hold this upload.")
	);
}

const ARTIFACT_HANDLERS: Record<string, ArtifactHandler> = {
	usage: (request) =>
		completed({
			device: EMPTY_BUDGET,
			project: request.project_id ? EMPTY_BUDGET : null,
			revisions: [],
		}),
	prune: (request) =>
		completed({
			project_id: request.project_id,
			pruned: request.revisions,
			freed_bytes: 0,
		}),
	status: (request, agent) => {
		const transfer = transferOf(agent, request);
		return "result" in transfer ? transfer : completed({ ...transfer });
	},
	abort: (request, agent) => {
		const transfer = transferOf(agent, request);
		if ("result" in transfer) return transfer;
		transfer.state = "aborted";
		return completed({ ...transfer });
	},
};

const artifact: CommandHandler = (command, { agent }) => {
	const request = (command.request ?? {}) as Record<string, unknown>;
	const kind = text(request.kind);
	return Object.hasOwn(ARTIFACT_HANDLERS, kind)
		? (ARTIFACT_HANDLERS[kind] as ArtifactHandler)(request, agent)
		: noDefault(`artifact · ${kind}`);
};

function removed<T>(
	list: (agent: FakeAgent) => T[],
	assign: (agent: FakeAgent, rows: T[]) => void,
	idField: "certificate_id" | "request_id",
): CommandHandler {
	return (command, { agent }) => {
		const id = command[idField];
		const rows = list(agent);
		const kept = rows.filter(
			(row) => (row as Record<string, unknown>)[idField] !== id,
		);
		if (kept.length === rows.length)
			return rejected("invalid", "Nothing to delete with this identifier.");
		assign(agent, kept);
		return completed({ [idField]: id, deleted: true });
	};
}

const apply: CommandHandler = (command, { agent }) => {
	const config = (command.config ?? {}) as Record<string, unknown>;
	const id = text(config.id);
	const existing = agent.placement(id);
	const expected = Number(command.expected_revision ?? 0);
	if ((existing?.config_revision ?? 0) !== expected)
		return revisionConflict(id);
	const revision = expected + 1;
	const row: PlacementStatusPlus = {
		...existing,
		id,
		project_id: text(config.project_id),
		deployment_id: text(config.deployment_id),
		revision: text(config.revision),
		config_revision: revision,
		max_replicas: Number(config.max_replicas ?? 1),
		desired_replicas: existing?.desired_replicas ?? 1,
		desired_state: "stopped",
		observed_state: "stopped",
		running_replicas: 0,
		ready_replicas: 0,
		intent_revision: (existing?.intent_revision ?? 0) + 1,
		applied_revision: existing?.applied_revision ?? null,
		replicas: undefined,
	};
	agent.placements = [
		...agent.placements.filter((other) => other.id !== id),
		row,
	];
	agent.configs.set(id, config);
	return completed({ placement_id: id, config_revision: revision });
};

const stageRollout: CommandHandler = (command, { agent, operationId }) => {
	const config = (command.config ?? {}) as Record<string, unknown>;
	const placement = agent.placement(config.id);
	const expected = Number(command.expected_revision ?? 0);
	if (!placement || placement.config_revision !== expected)
		return revisionConflict(config.id);
	const now = agent.now();
	const deadline = Number(command.deadline_seconds ?? 120);
	const staged: DeploymentRolloutStatus = {
		rollout_id: operationId,
		placement_id: placement.id,
		project_id: placement.project_id,
		state: "staged",
		failure_code: null,
		active_revision: expected + 1,
		base_revision: expected,
		previous_replicas: placement.desired_replicas ?? 1,
		candidate_replicas: placement.desired_replicas ?? 1,
		stabilization_seconds: Number(command.stabilization_seconds ?? 10),
		deadline_seconds: deadline,
		created_at: now,
		updated_at: now,
		deadline_at: now + deadline,
		stable_since: null,
	};
	agent.rollouts.push(staged);
	agent.staged.set(operationId, config);
	return completed(rolloutScope(staged));
};

const rolloutScope = (row: DeploymentRolloutStatus) => ({
	rollout_id: row.rollout_id,
	placement_id: row.placement_id,
	project_id: row.project_id,
	state: row.state,
});

function stagedRollout(agent: FakeAgent, command: Record<string, unknown>) {
	return agent.rollouts.find((row) => row.rollout_id === command.rollout_id);
}

const activateRollout: CommandHandler = (command, { agent }) => {
	const row = stagedRollout(agent, command);
	if (!row || row.state !== "staged")
		return rejected("invalid", "This update is not staged.");
	const placement = agent.placement(row.placement_id);
	if (placement) {
		placement.config_revision = Number(row.active_revision);
		placement.applied_revision = placement.config_revision;
		const config = agent.staged.get(row.rollout_id);
		if (config) agent.configs.set(placement.id, config);
	}
	row.state = "validating";
	agent.settling.add(row.rollout_id);
	return completed(rolloutScope(row));
};

const cancelRollout: CommandHandler = (command, { agent }) => {
	const row = stagedRollout(agent, command);
	if (!row || !["staged", "validating"].includes(row.state))
		return rejected("invalid", "This update can no longer be discarded.");
	row.state = "cancelled";
	return completed(rolloutScope(row));
};

const rolloutSecret: CommandHandler = (command, { agent }) => {
	const row = stagedRollout(agent, command);
	if (!row) return rejected("invalid", "Unknown rollout.");
	return completed({
		rollout_id: row.rollout_id,
		placement_id: row.placement_id,
		name: command.name,
		secret: "completed",
	});
};

const setSecret: CommandHandler = (command) =>
	completed({
		placement_id: command.placement_id,
		name: command.name,
		secret: "completed",
	});

const noDefault = (type: string) =>
	rejected(
		"failed",
		`The fake agent has no default answer for "${type}". Register one with api.agent(id).handle("${type.split(" ")[0]}", …).`,
	);

const DEFAULT_HANDLERS: Record<string, CommandHandler> = {
	inspect_page: inspectPage,
	metrics,
	project_metrics: metrics,
	logs: telemetry("logs"),
	messages: telemetry("messages"),
	placement_configuration: placementConfiguration,
	rollout,
	operation,
	certificates,
	certificate_requests: listed(
		"requests",
		(agent) => agent.certificateRequests,
		(row) => row.request_id,
		1,
	),
	certificate_issuers: listed(
		"issuers",
		(agent) => agent.certificateIssuers,
		(row) => row.certificate_id,
		4,
	),
	acme_certificates: listed(
		"policies",
		(agent) => agent.acme,
		(row) => row.certificate_id,
		4,
	),
	offline_queue: offlineQueue,
	telemetry_read: () => completed({ available: false, latest: 0 }),
	telemetry_roster_read: rosterRead(() => "telemetry"),
	archive_roster_read: rosterRead((command) => text(command.kind)),
	host_operation: (_command, { agent }) =>
		completed({ operation: agent.facts.host_operation ?? null }),
	rollout_history: rolloutHistory,
	operations,
	metrics_history: metricsHistory,
	offline_queue_operations: () => completed({ operations: [], next: null }),
	offline_queue_lookup: () =>
		completed({
			state: "applied",
			superseded_by: null,
			error: null,
			error_code: null,
		}),
	artifact,
	start: lifecycle(true),
	restart: lifecycle(true),
	stop: lifecycle(false),
	scale,
	remove,
	reboot: hostOperation("reboot"),
	update_agent: hostOperation("update_agent"),
	apply,
	stage_rollout: stageRollout,
	activate_rollout: activateRollout,
	cancel_rollout: cancelRollout,
	rollout_secret: rolloutSecret,
	set_secret: setSecret,
	offline_queue_retry: () => completed({}),
	offline_queue_skip: () => completed({}),
	delete_certificate: removed(
		(agent) => agent.certificates.certificates,
		(agent, rows) => {
			agent.certificates = {
				certificates: rows,
				inventory_revision: agent.certificates.inventory_revision + 1,
			};
		},
		"certificate_id",
	),
	delete_certificate_request: removed(
		(agent) => agent.certificateRequests,
		(agent, rows) => {
			agent.certificateRequests = rows;
		},
		"request_id",
	),
	delete_certificate_issuer: removed(
		(agent) => agent.certificateIssuers,
		(agent, rows) => {
			agent.certificateIssuers = rows;
		},
		"certificate_id",
	),
	delete_acme_certificate: removed(
		(agent) => agent.acme,
		(agent, rows) => {
			agent.acme = rows;
		},
		"certificate_id",
	),
};

function defaultHandler(type: string): CommandHandler {
	return Object.hasOwn(DEFAULT_HANDLERS, type)
		? (DEFAULT_HANDLERS[type] as CommandHandler)
		: () => noDefault(type);
}

/** Every command type the fake agent answers without a registered handler. */
export const FAKE_AGENT_COMMANDS: readonly string[] =
	Object.keys(DEFAULT_HANDLERS);

/* The hub. */

interface HubPolicy {
	jws: string | null;
	policy?: ManagementPolicy;
	version: number;
	digest: string | null;
	appliedVersion: number;
	appliedDigest: string | null;
}

interface HubBackup {
	public_key: Ed25519PublicKey;
	ciphertext: string;
	revision: number;
	updated_at: number;
}

export interface HubStream {
	kind: FleetAudience["kind"];
	scope: InventoryScope;
	grantId: string;
	sequence: number;
	bootId: string;
	/** Unix seconds. */
	observedAt: number;
	payload: Record<string, unknown>;
}

export interface HubArchive {
	archive_id: string;
	sequence: number;
	scope: string;
	kind: "logs" | "metrics";
	created_at: number;
	expires_at: number;
	manifest_jws: string;
	roster_jws: string;
	ciphertext: string;
	recipient_keys: { recipient_id: string; wrapped_key: string }[];
}

interface ReleaseTrust {
	manifest_url: string;
	public_keys: string[];
	minimum_sequence: number;
}

const READY_IDS = [
	"policy",
	"signing",
	"api",
	"signaling",
	"release",
	"database",
] as const;

const DEFAULT_LIMITS: DeviceLimits = {
	max_devices: 100,
	max_pending_enrollments: 10,
	enrollment_ttl_seconds: DAY,
	max_enrollments_per_day: 220,
	max_account_backups: 256,
};

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** The certificate id a test reminder's row carries. */
export const TEST_CERTIFICATE_ID = "00000000-0000-0000-0000-000000000000";
const TEST_NOTICE_WAIT_S = 600;

const HUB_ONLY_ROW_FIELDS = [
	"display_name",
	"relationship",
	"access_expires_at",
	"access_rules_expire_at",
	"revoked_at",
	"cloud_approvals",
	"auth_rejection",
] as const;

function releaseTrustOf(value: unknown): ReleaseTrust | null {
	const trust = value as Partial<ReleaseTrust> | null | undefined;
	if (!trust || typeof trust.manifest_url !== "string") return null;
	return {
		manifest_url: trust.manifest_url,
		public_keys: trust.public_keys ?? [],
		minimum_sequence: trust.minimum_sequence ?? 0,
	};
}

export class FakeHub {
	readonly me: string;
	readonly origin: string;
	readonly issuer: string;
	readonly apiBase: string;
	readonly password: string;
	version: HubVersion;
	/** Unix seconds. */
	now: () => number;
	rows = new Map<string, DeviceRow>();
	enrollments: HubEnrollment[] = [];
	/** Manifests of set-ups started through `POST devices/enrollments`, by device id. */
	manifests = new Map<string, OnboardingManifest>();
	policies = new Map<string, HubPolicy>();
	access = new Map<string, MyAccess>();
	backups = new Map<string, HubBackup>();
	certificates = new Map<string, PublicCertificateInventory>();
	notices = new Map<string, CertificateNotice[]>();
	/** Device id → certificate id (or `*`) → muted until (unix seconds, `null` = until unmuted). */
	mutes = new Map<string, Map<string, number | null>>();
	resources = new Map<string, DeviceResources>();
	/** Device id → controller key → reader declaration. */
	readers = new Map<string, Map<string, FleetReaderState>>();
	streams = new Map<string, HubStream[]>();
	/** Device id → controller key → saved observations. */
	inventory = new Map<string, Map<string, EncryptedInventory[]>>();
	archives = new Map<string, HubArchive[]>();
	eligibility = new Map<string, BillingEligibility>();
	/** Apps the signed-in account may not read (E20 answers 403). */
	hiddenApps = new Set<string>();
	/** The apps the hub can export to devices, by id; the sample apps by default. */
	apps: Record<string, AppInput> = { ...APPS };
	/** Event id → the variables its flow lets a service configure. */
	appVariables: Record<string, readonly DeploymentVariable[]> = {
		...VISITOR_PLAN_APP.variables,
		...CRM_PLAN_APP.variables,
	};
	/** Reminder channels the hub has a provider for; a test on another one is skipped as `not_configured`. */
	noticeChannels = { push: true, email: true };
	readiness: DeviceSetupReadiness;
	limits: DeviceLimits;
	usage?: DeviceUsage;
	archiveUsage?: ArchiveUsage;
	releaseTrust: ReleaseTrust | null;
	/** The signed release manifest and where `hubFetch` serves it; set by `publishRelease`. */
	release?: { url: string; jws: string };
	telemetryTiers: Record<
		string,
		{ max_bytes: number; retention_seconds: number }
	>;
	private readonly summarySeed?: ResourceSummary;
	private readonly lastTestNotice = new Map<string, number>();

	constructor(seed: DeviceSeed, options: FakeDeviceApiOptions) {
		this.me = seed.me;
		this.origin = options.origin ?? "https://hub.test";
		this.issuer = options.issuer ?? "https://auth.test";
		this.apiBase = `${this.origin}/api/v1`;
		this.password = options.password ?? FAKE_PASSWORD;
		this.version = options.hubVersion ?? "current";
		const clock = options.now ?? (() => seed.now * 1000);
		this.now = () => Math.floor(clock() / 1000);
		this.readiness = seed.readiness ?? {
			version: 1,
			ready: true,
			checks: READY_IDS.map((id) => ({
				id,
				ready: true,
				message: `${id} ready`,
			})),
		};
		this.limits = { ...(seed.usage?.limits ?? DEFAULT_LIMITS) };
		this.usage = seed.usage ? { ...seed.usage.usage } : undefined;
		this.archiveUsage = structuredClone(seed.archiveUsage);
		this.releaseTrust = releaseTrustOf(seed.releaseTrust);
		this.telemetryTiers = {
			FREE: { max_bytes: 0, retention_seconds: 0 },
			PRO: { max_bytes: 268_435_456, retention_seconds: 604_800 },
		};
		this.summarySeed = structuredClone(seed.resourceSummary);
		this.seedRegistry(seed);
		this.seedAccess(seed);
		this.seedStatus(seed);
		this.seedBackups(seed);
	}

	private seedRegistry(seed: DeviceSeed) {
		for (const row of seed.devices)
			this.rows.set(row.device_id, {
				...structuredClone(row),
				identity: fakeKeys.identity(row.device_id),
			});
		this.enrollments = (seed.pendingSetups ?? []).map((setup) => ({
			enrollment_id: setup.enrollmentId,
			device_id: setup.deviceId ?? crypto.randomUUID(),
			name: setup.name,
			state: setup.state,
			created_at: setup.createdAt ?? setup.expiresAt - DAY,
			expires_at: setup.expiresAt,
			controller_key_thumbprint: thumbprint(
				fakeKeys.controller(this.me, setup.deviceId ?? setup.enrollmentId),
			),
		}));
		for (const [id, inventory] of Object.entries(seed.certInventory))
			if (inventory) this.certificates.set(id, structuredClone(inventory));
		for (const [id, entry] of Object.entries(seed.resources))
			if (entry) this.resources.set(id, structuredClone(entry));
	}

	private seedAccess(seed: DeviceSeed) {
		for (const [id, access] of Object.entries(seed.myAccess ?? {}))
			if (access) this.access.set(id, structuredClone(access));
		for (const [id, view] of Object.entries(seed.policies))
			this.setPolicy(id, view.policy, view.version, view.applied_version);
		for (const [id, access] of this.access) {
			if (this.policies.has(id) || access.role !== "grantee") continue;
			this.setPolicy(
				id,
				this.granteePolicy(access),
				access.policy_version,
				access.applied_version,
			);
		}
	}

	/** The owner's access rules as far as a grantee's own access shows them. */
	private granteePolicy(access: MyAccess): ManagementPolicy {
		const now = this.now();
		return {
			version: 1,
			device_id: access.device_id,
			policy_version: access.policy_version,
			previous_policy_digest: null,
			grants: access.grants.map((grant) => ({
				grant_id: grant.grant_id,
				user_id: this.me,
				controller_key: fakeKeys.controller(this.me, access.device_id),
				scope: grant.scope,
				capabilities: grant.capabilities,
				expires_at: grant.expires_at,
				group_id: grant.group_id,
				group_version: null,
			})),
			issued_at: now - DAY,
			expires_at: access.policy_expires_at ?? now + 30 * DAY,
		};
	}

	private seedStatus(seed: DeviceSeed) {
		for (const [id, state] of Object.entries(seed.fleet)) {
			const grantId = this.grantIdOf(id);
			const facts = snapshotFacts(seed.live[id]?.inspection?.value);
			const streams: HubStream[] = [];
			for (const observation of state.status?.observations ?? [])
				streams.push({
					kind: "status",
					scope: observation.scope,
					grantId,
					sequence: state.status?.sequence ?? 1,
					bootId: observation.boot_id ?? "boot",
					observedAt: Math.floor((observation.observed_at ?? 0) / 1000),
					payload: {
						inspection: {
							device_id: id,
							boot_id: observation.boot_id,
							observed_at: observation.observed_at,
							placements: observation.placements.map(snapshotPlacement),
							...facts,
						},
					},
				});
			for (const sample of state.metrics ?? [])
				streams.push({
					kind: "metrics",
					scope: sample.scope,
					grantId,
					sequence: state.status?.sequence ?? 1,
					bootId: state.status?.bootId ?? "boot",
					observedAt: sample.observedAt,
					payload: { metrics: { records: [], ...sample.sample } },
				});
			if (streams.length) this.streams.set(id, streams);
			if (state.reader) this.seedReader(id, state.reader);
			if (state.saved) this.seedSaved(id, state.saved.observations);
		}
	}

	private seedReader(
		id: string,
		reader: { revision: number; expiresAt: number },
	) {
		const key = fakeKeys.controller(this.me, id);
		this.readers.set(
			id,
			new Map([
				[
					key.x,
					{
						reader_jws: fakeCompact(
							{
								api_base_url: this.apiBase,
								user_id: this.me,
								device_id: id,
								controller_key: key,
								revision: reader.revision,
								issued_at: reader.expiresAt - 365 * DAY,
								expires_at: reader.expiresAt,
							},
							key,
						),
						revision: reader.revision,
						deleted: false,
					},
				],
			]),
		);
	}

	private seedSaved(
		id: string,
		observations: NonNullable<
			DeviceSeed["fleet"][string]["saved"]
		>["observations"],
	) {
		const key = fakeKeys.controller(this.me, id);
		this.inventory.set(
			id,
			new Map([
				[
					key.x,
					observations.map((observation) => ({
						binding: {
							issuer: this.issuer,
							api_origin: this.origin,
							account_id: this.me,
							device_id: id,
							controller_key: key,
							scope: observation.scope,
							revision: 1,
						},
						ciphertext: base64url(
							utf8(
								JSON.stringify({
									device_id: id,
									boot_id: observation.boot_id,
									observed_at: observation.observed_at,
									placements: observation.placements.map(snapshotPlacement),
								}),
							),
						),
					})),
				],
			]),
		);
	}

	private seedBackups(seed: DeviceSeed) {
		for (const [id, backup] of Object.entries(seed.accountBackups)) {
			if (!backup || backup.revision < 1) continue;
			const vault = this.localVault(id);
			const context: AccountRecoveryContext = {
				issuer: this.issuer,
				account: this.me,
				api_origin: this.origin,
				device_id: id,
				controller_key: vault.controllerPublic.controller_key,
				revision: backup.revision,
			};
			const sealed = sealFakeRecovery(
				context,
				utf8(this.password),
				backupEnvelope(this.origin, vault),
			);
			this.backups.set(id, {
				public_key: vault.controllerPublic.controller_key,
				ciphertext: base64url(Uint8Array.from(sealed.ciphertext)),
				revision: backup.revision,
				updated_at: backup.updatedAt ?? this.now() - DAY,
			});
		}
	}

	/* Identity. */

	ownerOf(deviceId: string): string {
		return this.rows.get(deviceId)?.owner_id ?? this.me;
	}

	ownerInvitationKey(deviceId: string): Ed25519PublicKey {
		return this.manifest(deviceId).owner_invitation_key;
	}

	/** The grant this account's keys act under: `owner`, or its sharing grant. */
	grantIdOf(deviceId: string): string {
		if (this.ownerOf(deviceId) === this.me) return "owner";
		return this.access.get(deviceId)?.grants[0]?.grant_id ?? "shared-grant";
	}

	manifest(
		deviceId: string,
		details: { ownerId?: string; name?: string; registeredAt?: number } = {},
	): OnboardingManifest {
		const enrolled = this.manifests.get(deviceId);
		if (enrolled) return enrolled;
		const row = this.rows.get(deviceId);
		const ownerId = details.ownerId ?? row?.owner_id ?? this.me;
		const registeredAt =
			details.registeredAt ?? row?.registered_at ?? this.now() - DAY;
		return {
			version: 1,
			enrollment_id: `enrollment-${deviceId}`,
			device_id: deviceId,
			owner_id: ownerId,
			name: details.name ?? row?.name ?? deviceId,
			api_base_url: this.apiBase,
			bootstrap_key: fakeKeys.bootstrap(deviceId),
			controller_key: fakeKeys.controller(ownerId, deviceId),
			owner_invitation_key: fakeKeys.invitation(ownerId, deviceId),
			issued_at: registeredAt - 60,
			expires_at: registeredAt + DAY,
		};
	}

	receipt(deviceId: string): DeviceReceipt {
		const row = this.visible(deviceId);
		const manifest = this.manifest(deviceId);
		return {
			enrollment_id: manifest.enrollment_id,
			device_id: deviceId,
			owner_id: row.owner_id,
			name: row.name,
			identity: row.identity,
			manifest_jws: fakeCompact(manifest),
			binding_jws: fakeCompact({ device_id: deviceId, identity: row.identity }),
			registered_at: row.registered_at,
			auth_epoch: row.auth_epoch,
		};
	}

	/**
	 * The keys this computer would hold for a device, opening with the hub's
	 * password. A device the account does not own gets a shared-access vault.
	 */
	localVault(
		deviceId: string,
		options: {
			ownerId?: string;
			name?: string;
			grantId?: string;
			restored?: boolean;
			password?: string;
		} = {},
	): LocalDeviceVault {
		const ownerId = options.ownerId ?? this.ownerOf(deviceId);
		const owner = ownerId === this.me;
		const check = fakePasswordCheck(
			deviceId,
			utf8(options.password ?? this.password),
		);
		const controller = fakeControllerPublic(this.me, deviceId);
		return {
			deviceId,
			controllerPublic: controller,
			controllerVault: sealFakeVault({
				fake: "controller",
				device_id: deviceId,
				check,
				controller,
			}),
			...(owner
				? {
						invitationVault: sealFakeVault({
							fake: "invitation",
							device_id: deviceId,
							check,
							public_key: fakeKeys.invitation(this.me, deviceId),
						}),
					}
				: { ownerControllerKey: fakeKeys.controller(ownerId, deviceId) }),
			manifestJws: fakeCompact(
				this.manifest(deviceId, { ownerId, name: options.name }),
			),
			grantId: owner ? "owner" : (options.grantId ?? this.grantIdOf(deviceId)),
			...(options.restored ? { requiresFreshEndpoint: true } : {}),
		};
	}

	/* Rows. */

	visible(deviceId: string): DeviceRow {
		const row = this.rows.get(deviceId);
		if (!row) throw notFound("Device");
		return row;
	}

	owned(deviceId: string, active = true): DeviceRow {
		const row = this.rows.get(deviceId);
		if (!row || row.owner_id !== this.me || (active && row.status !== "active"))
			throw notFound("Device");
		return row;
	}

	private relationshipOf(
		row: DeviceRow,
	): NonNullable<DeviceRow["relationship"]> {
		if (row.owner_id === this.me) return "owner";
		if (row.relationship) return row.relationship;
		return row.cloud_approvals && !this.access.has(row.device_id)
			? "cloud_approval"
			: "shared";
	}

	/** `GET /devices/{id}`: a shared device that was revoked or whose access ended reads as missing. */
	readable(deviceId: string): DeviceRow {
		const row = this.visible(deviceId);
		const ended =
			row.access_expires_at != null && row.access_expires_at <= this.now();
		if (
			this.relationshipOf(row) === "shared" &&
			(row.status === "revoked" || ended)
		)
			throw notFound("Device");
		return row;
	}

	/** A row as this hub version sends it: the current hub always sends every key, `null` when unset. */
	view(row: DeviceRow): DeviceRow {
		const copy = structuredClone(row);
		if (this.version === "current")
			return {
				...copy,
				display_name: copy.display_name ?? null,
				relationship: this.relationshipOf(copy),
				access_expires_at: copy.access_expires_at ?? null,
				access_rules_expire_at: copy.access_rules_expire_at ?? null,
				revoked_at: copy.revoked_at ?? null,
				cloud_approvals: copy.cloud_approvals ?? null,
				auth_rejection: copy.auth_rejection ?? null,
			};
		const hidden = new Set<string>(HUB_ONLY_ROW_FIELDS);
		return Object.fromEntries(
			Object.entries(copy).filter(([field]) => !hidden.has(field)),
		) as unknown as DeviceRow;
	}

	/** `GET /devices`: the account's own devices first (active before revoked), then shared, then cloud approvals; newest first within each. */
	listRows(): DeviceRow[] {
		const rank = (row: DeviceRow) => {
			const relationship = this.relationshipOf(row);
			if (relationship !== "owner") return relationship === "shared" ? 2 : 3;
			return row.status === "active" ? 0 : 1;
		};
		return [...this.rows.values()]
			.sort((a, b) => rank(a) - rank(b) || b.registered_at - a.registered_at)
			.map((row) => this.view(row));
	}

	/** The device checks in now (a heartbeat). */
	checkIn(deviceId: string, at = this.now()) {
		this.visible(deviceId).last_seen_at = at;
	}

	/** The device finishes a pending set-up: its row appears, never checked in yet. */
	redeem(enrollmentId: string): DeviceRow {
		const enrollment = this.enrollments.find(
			(row) => row.enrollment_id === enrollmentId,
		);
		if (!enrollment) throw notFound("Enrollment");
		this.enrollments = this.enrollments.filter((row) => row !== enrollment);
		const row: DeviceRow = {
			device_id: enrollment.device_id,
			owner_id: this.me,
			name: enrollment.name,
			status: "active",
			registered_at: this.now(),
			last_seen_at: null,
			auth_epoch: 1,
			identity: fakeKeys.identity(enrollment.device_id),
			display_name: null,
			relationship: "owner",
			access_expires_at: null,
			access_rules_expire_at: null,
			revoked_at: null,
			cloud_approvals: null,
			auth_rejection: null,
		};
		this.rows.set(row.device_id, row);
		return row;
	}

	/* Access rules. */

	setPolicy(
		deviceId: string,
		policy: ManagementPolicy | undefined,
		version = policy?.policy_version ?? 0,
		appliedVersion = version,
	) {
		const jws = policy
			? fakeCompact(policy, this.ownerInvitationKey(deviceId))
			: null;
		const digest = jws ? fakeDigest(jws) : null;
		this.policies.set(deviceId, {
			jws,
			policy,
			version,
			digest,
			appliedVersion,
			appliedDigest:
				appliedVersion === version
					? digest
					: appliedVersion > 0
						? fakeDigest(`policy:${deviceId}:${appliedVersion}`)
						: null,
		});
	}

	/** The owner publishes access rules: verified with the owner's key, newer than the stored ones, not applied yet. */
	putPolicy(deviceId: string, policyJws: string): PolicyView {
		this.owned(deviceId);
		const policy = verified(() =>
			readFakeCompact<ManagementPolicy>(
				policyJws,
				this.ownerInvitationKey(deviceId),
			),
		);
		const stored = this.policyView(deviceId);
		if (policy.device_id !== deviceId)
			throw badRequest("Invalid policy device");
		if (policy.policy_version <= stored.version)
			throw conflict("The access rules did not move forward");
		this.setPolicy(
			deviceId,
			policy,
			policy.policy_version,
			stored.applied_version,
		);
		return this.policyView(deviceId);
	}

	/** The device confirms the newest access rules. */
	applyPolicy(deviceId: string) {
		const entry = this.policies.get(deviceId);
		if (!entry) return;
		entry.appliedVersion = entry.version;
		entry.appliedDigest = entry.digest;
	}

	policyView(deviceId: string): PolicyView {
		const entry = this.policies.get(deviceId);
		return {
			policy_jws: entry?.jws ?? null,
			version: entry?.version ?? 0,
			digest: entry?.digest ?? null,
			applied_version: entry?.appliedVersion ?? 0,
			applied_digest: entry?.appliedDigest ?? null,
		};
	}

	myAccess(deviceId: string): MyAccess {
		const row = this.visible(deviceId);
		const seeded = this.access.get(deviceId);
		if (seeded) return structuredClone(seeded);
		if (row.owner_id !== this.me)
			throw forbidden("Your access to this device has ended");
		const entry = this.policies.get(deviceId);
		return {
			device_id: deviceId,
			role: "owner",
			owner_id: this.me,
			policy_version: entry?.version ?? 0,
			policy_expires_at: entry?.policy?.expires_at ?? null,
			applied_version: entry?.appliedVersion ?? 0,
			applied: (entry?.appliedVersion ?? 0) === (entry?.version ?? 0),
			grants: [],
		};
	}

	/* Encrypted status. */

	reader(deviceId: string, key: string): FleetReaderState | undefined {
		return this.readers.get(deviceId)?.get(key);
	}

	putReader(
		deviceId: string,
		key: string,
		readerJws: string,
	): FleetReaderState {
		const declared = readFakeCompact<{ revision: number }>(readerJws, {
			kty: "OKP",
			crv: "Ed25519",
			x: key,
		});
		const current = this.reader(deviceId, key);
		if (current && declared.revision <= current.revision)
			throw conflict("The reader declaration is not newer than the stored one");
		const state = {
			reader_jws: readerJws,
			revision: declared.revision,
			deleted: false,
		};
		const readers = this.readers.get(deviceId) ?? new Map();
		readers.set(key, state);
		this.readers.set(deviceId, readers);
		return state;
	}

	fleetView(deviceId: string, key: string): FleetView {
		const row = this.visible(deviceId);
		const reader = this.reader(deviceId, key);
		if (!reader || reader.deleted) throw notFound("Fleet reader");
		const grantId = this.grantIdOf(deviceId);
		const policy = this.policies.get(deviceId);
		const readerDigest = fakeDigest(`reader:${key}`);
		const snapshots: EncryptedFleetSnapshot[] = (
			this.streams.get(deviceId) ?? []
		)
			.filter((stream) => stream.grantId === grantId)
			.map((stream) => ({
				manifest_jws: fakeCompact(
					{
						device_id: deviceId,
						sequence: stream.sequence,
						audience: {
							reader_digest: readerDigest,
							grant_id: stream.grantId,
							scope: stream.scope,
							kind: stream.kind,
							policy_digest:
								grantId === "owner" ? null : (policy?.digest ?? null),
						} satisfies FleetAudience,
						observed_at: stream.observedAt,
						boot_id: stream.bootId,
					},
					row.identity.telemetry_key,
				),
				ciphertext: base64url(utf8(JSON.stringify(stream.payload))),
			}));
		return {
			snapshots,
			policy_jws: policy?.jws ?? null,
			reader_jws: reader.reader_jws,
		};
	}

	/** The device publishes a status snapshot: the next sequence, observed now. */
	publishStatus(deviceId: string, agent: FakeAgent) {
		const now = this.now();
		const streams = this.streams.get(deviceId) ?? [];
		const previous = streams.find(
			(stream) => stream.kind === "status" && stream.scope.kind === "device",
		);
		const next: HubStream = {
			kind: "status",
			scope: { kind: "device" },
			grantId: this.grantIdOf(deviceId),
			sequence: (previous?.sequence ?? 0) + 1,
			bootId: agent.bootId,
			observedAt: now,
			payload: {
				inspection: {
					device_id: deviceId,
					boot_id: agent.bootId,
					observed_at: now * 1000,
					placements: [...agent.placements]
						.sort((a, b) => (a.id < b.id ? -1 : 1))
						.map(snapshotPlacement),
					...pick(agent.facts as Record<string, unknown>, [
						"agent",
						"host",
						"host_operation",
					]),
				},
			},
		};
		this.streams.set(deviceId, [
			...streams.filter((stream) => stream !== previous),
			next,
		]);
		this.checkIn(deviceId, now);
	}

	inventoryView(deviceId: string, key: string): InventoryView {
		const row = this.visible(deviceId);
		const grants = this.access.get(deviceId)?.grants;
		const scopes: InventoryScope[] =
			row.owner_id === this.me || !grants?.length
				? [{ kind: "device" }]
				: grants.map((grant) => grant.scope);
		return {
			scopes,
			observations: structuredClone(
				this.inventory.get(deviceId)?.get(key) ?? [],
			),
		};
	}

	putInventory(
		deviceId: string,
		key: string,
		encrypted: EncryptedInventory,
	): InventoryView {
		this.visible(deviceId);
		if (!encrypted?.binding?.scope || typeof encrypted.ciphertext !== "string")
			throw badRequest("Invalid encrypted inventory");
		const scopeKey = JSON.stringify(encrypted.binding.scope);
		const stored = this.inventory.get(deviceId) ?? new Map();
		const rows: EncryptedInventory[] = stored.get(key) ?? [];
		const previous = rows.find(
			(row) => JSON.stringify(row.binding.scope) === scopeKey,
		);
		if (previous && encrypted.binding.revision <= previous.binding.revision)
			throw conflict("Saved inventory did not move forward");
		stored.set(key, [...rows.filter((row) => row !== previous), encrypted]);
		this.inventory.set(deviceId, stored);
		return this.inventoryView(deviceId, key);
	}

	/* Cloud approvals. */

	resourcesOf(deviceId: string): DeviceResources {
		this.visible(deviceId);
		let entry = this.resources.get(deviceId);
		if (!entry) {
			entry = { grants: [], billing: [], instances: [] };
			this.resources.set(deviceId, entry);
		}
		return entry;
	}

	/** A grant as this hub version sends it (E15 adds the effective end). */
	grantView(grant: ResourceGrant): ResourceGrant {
		if (this.version === "old") return structuredClone(grant);
		return {
			...structuredClone(grant),
			effective_expires_at: grant.effective_expires_at ?? grant.expires_at,
			effective_limit: grant.effective_limit ?? "approval",
			approved_by_user_id:
				grant.approved_by_user_id ?? grant.delegating_user_id,
		};
	}

	grant(deviceId: string, grantId: string): ResourceGrant {
		const grant = this.resourcesOf(deviceId).grants.find(
			(row) => row.grant_id === grantId,
		);
		if (!grant) throw notFound("Cloud approval");
		return grant;
	}

	billing(deviceId: string, billingId: string): BillingGrant {
		const billing = this.resourcesOf(deviceId).billing.find(
			(row) => row.billing_grant_id === billingId,
		);
		if (!billing) throw notFound("Spending approval");
		return billing;
	}

	resourceSummary(): ResourceSummary {
		const seeded = new Map(
			(this.summarySeed?.devices ?? []).flatMap((device) =>
				device.approvals.map((row) => [row.grant_id, row] as const),
			),
		);
		return {
			server_time: this.now(),
			devices: [...this.resources.entries()].map(([deviceId, entry]) => ({
				device_id: deviceId,
				approvals: entry.grants.map((grant) => {
					const known = seeded.get(grant.grant_id);
					return {
						grant_id: grant.grant_id,
						placement_id: grant.placement_id,
						app_id: grant.app_id,
						status: grant.status,
						expires_at: grant.expires_at,
						effective_expires_at:
							known?.effective_expires_at ?? grant.expires_at,
						effective_limit: known?.effective_limit ?? "approval",
						online_access: grant.online_access ?? null,
						online_write_blocked: known?.online_write_blocked ?? null,
						payer_is_me: entry.billing.some(
							(billing) =>
								billing.grant_id === grant.grant_id &&
								billing.payer_id === this.me,
						),
						approver_is_me: grant.delegating_user_id === this.me,
					};
				}),
				billing: entry.billing
					.filter((billing) => billing.status === "active")
					.map((billing) => ({
						billing_grant_id: billing.billing_grant_id,
						grant_id: billing.grant_id,
						limit_micros: billing.limit_micros,
						used_micros: billing.used_micros,
						reserved_micros: billing.reserved_micros,
						expires_at: billing.expires_at,
						payer_is_me: billing.payer_id === this.me,
					})),
			})),
		};
	}

	appPlacements(appId: string): AppDevicePlacements {
		if (this.hiddenApps.has(appId))
			throw forbidden("You do not have access to this app");
		const now = this.now();
		const placements: AppDevicePlacements["placements"] = [];
		for (const [deviceId, entry] of this.resources) {
			const row = this.rows.get(deviceId);
			for (const grant of entry.grants) {
				if (grant.app_id !== appId) continue;
				const billing = entry.billing.find(
					(candidate) =>
						candidate.grant_id === grant.grant_id &&
						candidate.status === "active",
				);
				const leases = entry.instances.filter(
					(lease) =>
						lease.grant_id === grant.grant_id && lease.lease_expires_at > now,
				);
				placements.push({
					device_id: deviceId,
					placement_id: grant.placement_id,
					deployment_id: grant.deployment_id,
					relationship:
						row?.relationship ??
						(row?.owner_id === this.me ? "owner" : "cloud_approval"),
					grant: {
						grant_id: grant.grant_id,
						status: grant.status,
						expires_at: grant.expires_at,
						effective_expires_at:
							grant.effective_expires_at ?? grant.expires_at,
						effective_limit: grant.effective_limit ?? "approval",
						online_access: grant.online_access ?? null,
						model_ids: grant.model_ids,
						max_instances: grant.max_instances,
						approved_by_user_id:
							grant.approved_by_user_id ?? grant.delegating_user_id,
						created_at: grant.created_at ?? null,
					},
					billing: billing
						? {
								billing_grant_id: billing.billing_grant_id,
								limit_micros: billing.limit_micros,
								used_micros: billing.used_micros,
								reserved_micros: billing.reserved_micros,
								expires_at: billing.expires_at,
								payer_is_me: billing.payer_id === this.me,
							}
						: null,
					instances: {
						active: leases.length,
						newest_lease_expires_at: leases.length
							? Math.max(...leases.map((lease) => lease.lease_expires_at))
							: null,
					},
				});
			}
		}
		return { server_time: now, placements };
	}

	/** An online app's executable definitions as the owner approves them: its deployable events, one flow each. */
	deviceMetadata(appId: string) {
		const app = this.apps[appId];
		if (!app || this.hiddenApps.has(appId))
			throw forbidden("You do not have access to this app");
		if (app.visibility === "Offline")
			throw badRequest("Only online projects use executable metadata export");
		const events = app.events.filter(
			(event) => eventEligibility(event).eligible,
		);
		const boardOf = (eventId: string) => `board_${eventId}`;
		const documents: Record<string, unknown> = {};
		for (const event of events) {
			const boardId = boardOf(event.id);
			documents[
				`boards/${boardId}/versions/${event.board_version?.join("/")}`
			] = {
				id: boardId,
				version: event.board_version,
				variables: Object.fromEntries(
					(this.appVariables[event.id] ?? []).map((variable) => [
						variable.id,
						{ ...variable, exposed: true },
					]),
				),
				layers: {},
			};
			documents[
				`events/${event.id}/versions/${event.event_version?.join("/")}`
			] = {
				id: event.id,
				name: event.name,
				active: event.active,
				event_type: event.event_type,
				event_version: event.event_version,
				board_version: event.board_version,
				board_id: boardId,
				default_page_id: event.default_page_id ?? null,
				variants: [],
			};
		}
		documents.app = {
			id: app.id,
			visibility: app.visibility,
			events: events.map((event) => event.id),
			boards: events.map((event) => boardOf(event.id)),
			bits: [],
		};
		return { version: 1, project_id: app.id, documents };
	}

	billingUsage(deviceId: string, billingId: string): BillingUsage {
		const billing = this.billing(deviceId, billingId);
		const leases = this.resourcesOf(deviceId).instances.filter(
			(lease) => lease.billing_grant_id === billingId,
		);
		const share = (total: number, index: number) => (index === 0 ? total : 0);
		return {
			billing_grant_id: billingId,
			totals: {
				used_micros: billing.used_micros,
				reserved_micros: billing.reserved_micros,
				operations: leases.length,
			},
			instances: leases.map((lease, index) => ({
				instance_id: lease.instance_id,
				used_micros: share(billing.used_micros, index),
				reserved_micros: share(billing.reserved_micros, index),
				operations: 1,
				first_at: lease.registered_at,
				last_at: lease.registered_at,
			})),
		};
	}

	billingEligibility(deviceId: string, grantId: string): BillingEligibility {
		const grant = this.grant(deviceId, grantId);
		return (
			this.eligibility.get(grantId) ?? {
				payer_id: this.me,
				plan: "PRO",
				eligible: true,
				models: grant.model_ids.map((modelId) => ({
					model_id: modelId,
					tier: "PRO",
					allowed: true,
				})),
			}
		);
	}

	/* Account-wide reads. */

	/** `open`: pending and lapsed set-ups (reported `expired`, kept 7 days); `recent` adds cancelled ones. */
	enrollmentList(state: string): HubEnrollment[] {
		if (state !== "open" && state !== "recent")
			throw badRequest("Unknown enrollment state filter");
		const now = this.now();
		return this.enrollments
			.filter((row) => state === "recent" || row.state === "pending")
			.filter((row) => row.expires_at > now - 7 * DAY)
			.map((row) => ({
				...row,
				state:
					row.state === "pending" && row.expires_at <= now
						? ("expired" as const)
						: row.state,
			}))
			.sort((a, b) => b.created_at - a.created_at)
			.slice(0, 200);
	}

	usageView() {
		const rows = [...this.rows.values()].filter(
			(row) => row.owner_id === this.me,
		);
		const now = this.now();
		const pending = this.enrollments.filter(
			(row) => row.state === "pending" && row.expires_at > now,
		);
		return {
			server_time: now,
			limits: { ...this.limits },
			usage: this.usage ?? {
				active_devices: rows.filter((row) => row.status === "active").length,
				revoked_devices: rows.filter((row) => row.status === "revoked").length,
				pending_enrollments: pending.length,
				enrollments_last_24h: this.enrollments.filter(
					(row) => row.created_at > now - DAY,
				).length,
				account_backups: this.backups.size,
			},
		};
	}

	backupList(): AccountBackupList {
		return {
			vaults: [...this.backups.entries()]
				.map(([keyId, backup]) => ({
					key_id: keyId,
					revision: backup.revision,
					updated_at: backup.updated_at,
					public_key_thumbprint: thumbprint(backup.public_key),
				}))
				.sort((a, b) => b.updated_at - a.updated_at),
			used: this.backups.size,
			max: this.limits.max_account_backups,
		};
	}

	putBackup(
		keyId: string,
		write: {
			public_key: Ed25519PublicKey;
			ciphertext: string;
			revision: number;
		},
	) {
		if (
			!write?.public_key?.x ||
			typeof write.ciphertext !== "string" ||
			!Number.isSafeInteger(write.revision)
		)
			throw badRequest("Invalid account backup");
		const current = this.backups.get(keyId);
		if (!current && this.backups.size >= this.limits.max_account_backups)
			throw hubError(
				429,
				"TOO_MANY_REQUESTS",
				"Controller backup storage limit reached",
			);
		if (current && write.revision <= current.revision)
			throw conflict("The account backup did not move forward");
		this.backups.set(keyId, {
			public_key: write.public_key,
			ciphertext: write.ciphertext,
			revision: write.revision,
			updated_at: this.now(),
		});
		return { revision: write.revision };
	}

	/** E13: every active device of the account, reported or not; a shared device only with certificate access. */
	fleetCertificates() {
		const unreported = { revision: 0, updated_at: null, certificates: [] };
		return [...this.rows.values()]
			.filter(
				(row) =>
					row.status === "active" &&
					(row.owner_id === this.me || this.certificates.has(row.device_id)),
			)
			.map((row) => ({
				device_id: row.device_id,
				...structuredClone(this.certificates.get(row.device_id) ?? unreported),
			}));
	}

	/** E9: the caller's reminder rows, newest first; a certificate filter never returns test rows. */
	noticeList(deviceId: string, certificate: string | null, limit: number) {
		this.visible(deviceId);
		const bounded = Math.min(
			200,
			Math.max(1, Number.isFinite(limit) ? limit : 50),
		);
		return (this.notices.get(deviceId) ?? [])
			.filter((row) => !certificate || row.certificate_id === certificate)
			.slice(0, bounded);
	}

	private mutesOf(deviceId: string): Map<string, number | null> {
		this.visible(deviceId);
		const mutes = this.mutes.get(deviceId) ?? new Map<string, number | null>();
		this.mutes.set(deviceId, mutes);
		return mutes;
	}

	/** E10: `certificate_id: null` mutes the whole device; an unreported certificate is a coded 404. */
	mute(deviceId: string, certificateId: string | null, until: number | null) {
		const mutes = this.mutesOf(deviceId);
		if (until !== null && until <= this.now())
			throw badRequest("A mute ends in the future");
		if (certificateId !== null) {
			if (!UUID.test(certificateId)) throw badRequest("Invalid certificate id");
			const known = this.certificates
				.get(deviceId)
				?.certificates.some((row) => row.certificate_id === certificateId);
			if (!known)
				throw notFound("The device has not reported this certificate");
		}
		mutes.set(certificateId ?? "*", until);
		return { certificate_id: certificateId, until };
	}

	unmute(deviceId: string, certificate: string | null) {
		this.mutesOf(deviceId).delete(certificate ?? "*");
	}

	muteList(deviceId: string) {
		const now = this.now();
		return [...this.mutesOf(deviceId).entries()]
			.filter(([, until]) => until === null || until > now)
			.map(([certificate, until]) => ({
				certificate_id: certificate === "*" ? null : certificate,
				until,
			}));
	}

	/** E11: one test per device every 10 minutes; a test that sent nothing starts no wait. */
	testNotice(deviceId: string, channel: string) {
		this.visible(deviceId);
		const now = this.now();
		const waited =
			now - (this.lastTestNotice.get(deviceId) ?? Number.NEGATIVE_INFINITY);
		if (waited < TEST_NOTICE_WAIT_S)
			throw Object.assign(
				hubError(
					429,
					"TOO_MANY_REQUESTS",
					"A test reminder was sent less than ten minutes ago",
				),
				{ retryAfter: TEST_NOTICE_WAIT_S - waited },
			);
		const wanted = channel === "both" ? ["push", "email"] : [channel];
		const configured = (name: string) =>
			(name === "push" && this.noticeChannels.push) ||
			(name === "email" && this.noticeChannels.email);
		const sent = wanted.filter(configured);
		const skipped = wanted
			.filter((name) => !configured(name))
			.map((name) => ({ channel: name, reason: "not_configured" }));
		if (sent.length) this.lastTestNotice.set(deviceId, now);
		this.notices.set(deviceId, [
			...sent.map((name) => ({
				certificate_id: TEST_CERTIFICATE_ID,
				certificate_revision: 0,
				not_after: 0,
				stage: "test",
				channel: name,
				status: "sent",
				attempts: 1,
				completed_at: now,
				next_attempt_at: null,
			})),
			...(this.notices.get(deviceId) ?? []),
		]);
		return { sent, skipped };
	}

	/* Agent releases. */

	/**
	 * Publishes a signed agent release (a real Ed25519 signature, as the client
	 * verifies it) and makes the hub trust its key. The manifest is served by
	 * `hubFetch` at the release trust's manifest URL.
	 */
	async publishRelease(release: {
		version: string;
		sequence?: number | null;
	}): Promise<void> {
		const pair = (await crypto.subtle.generateKey("Ed25519", true, [
			"sign",
			"verify",
		])) as CryptoKeyPair;
		const key = base64url(
			new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey)),
		);
		const url =
			this.releaseTrust?.manifest_url ?? `${this.origin}/releases/release.jws`;
		const sequence = release.sequence ?? 1;
		const now = this.now();
		const header = {
			alg: "EdDSA",
			typ: "flow-like-standalone-release+jws",
			kid: fakeDigest(JSON.stringify({ crv: "Ed25519", kty: "OKP", x: key })),
		};
		const manifest = {
			version: 1,
			state_schema_version: 12,
			sequence,
			release_version: release.version,
			issued_at: now - 3_600,
			expires_at: now + 7 * DAY,
			artifacts: [
				{
					target: "x86_64-unknown-linux-gnu",
					url: `${new URL(url).origin}/standalone/${release.version}/x86_64-unknown-linux-gnu`,
					size: 1_024,
					sha256: "0".repeat(64),
				},
			],
			container: null,
		};
		const signed = [header, manifest]
			.map((part) => base64url(utf8(JSON.stringify(part))))
			.join(".");
		const signature = await crypto.subtle.sign(
			"Ed25519",
			pair.privateKey,
			utf8(signed),
		);
		this.release = {
			url,
			jws: `${signed}.${base64url(new Uint8Array(signature))}`,
		};
		this.releaseTrust = {
			manifest_url: url,
			public_keys: [key],
			minimum_sequence: Math.min(
				this.releaseTrust?.minimum_sequence ?? sequence,
				sequence,
			),
		};
	}

	/* The unauthenticated hub record. */

	record(mode: HubMode): Record<string, unknown> {
		const limits =
			this.version === "old"
				? {}
				: {
						max_devices_per_user: this.limits.max_devices,
						max_pending_enrollments_per_user:
							this.limits.max_pending_enrollments,
						enrollment_ttl_seconds: this.limits.enrollment_ttl_seconds,
					};
		return {
			name: "Fake hub",
			domain: new URL(this.origin).host,
			standalone: {
				enabled: mode.devicesEnabled,
				...limits,
				api_base_url: this.apiBase,
				telemetry_tiers: this.telemetryTiers,
				release_trust: this.releaseTrust,
			},
		};
	}
}

function thumbprint(key: Ed25519PublicKey): string {
	return fakeDigest(`thumbprint:${key.x}`);
}

/** Device-scope facts a status snapshot carries: release, boot time, unhealthy tasks, host operation. */
function snapshotFacts(
	inspection: InspectionPlus | undefined,
): Record<string, unknown> {
	if (!inspection) return {};
	const tasks = inspection.tasks
		?.filter((task) => task.state !== "ok")
		.map(({ consecutive_failures: _failures, ...task }) => task);
	return {
		...pick(inspection, ["agent", "host"]),
		...(tasks?.length ? { tasks } : {}),
		...(inspection.hostOperation === undefined
			? {}
			: { host_operation: inspection.hostOperation }),
	};
}

/* Hub routes. */

interface Route {
	method: HubMethod;
	template: string;
	/** Added by this work: absent on an old hub. */
	added?: true;
	handle(request: HubRequest): unknown;
}

const body = <T>(request: HubRequest) => (request.body ?? {}) as T;

/** The request must state `display_name` and nothing else; the hub's JSON extractor answers 422 without a code. */
function displayName(request: unknown): string | null {
	const fields = Object.keys((request ?? {}) as object);
	const value = (request as { display_name?: unknown } | null)?.display_name;
	const stated = value === null || typeof value === "string";
	if (fields.length !== 1 || fields[0] !== "display_name" || !stated)
		throw hubError(
			422,
			undefined,
			"Failed to deserialize the JSON body into the target type",
		);
	if (value === null) return null;
	const name = value.normalize("NFC").trim();
	if (!name || [...name].length > 64 || /\p{Cc}/u.test(name))
		throw badRequest("A device name has 1 to 64 characters");
	return name;
}

function registryRoutes(hub: FakeHub): Route[] {
	return [
		{
			method: "GET",
			template: "devices",
			handle: () => hub.listRows(),
		},
		{ method: "GET", template: "devices/setup", handle: () => hub.readiness },
		{
			method: "GET",
			template: "devices/usage",
			added: true,
			handle: () => hub.usageView(),
		},
		{
			method: "GET",
			template: "devices/enrollments",
			added: true,
			handle: ({ query }) => hub.enrollmentList(query.get("state") ?? "open"),
		},
		{
			method: "POST",
			template: "devices/enrollments",
			handle: (request) => createEnrollment(hub, body(request)),
		},
		{
			method: "DELETE",
			template: "devices/enrollments/:enrollment",
			handle: ({ params }) => {
				const row = hub.enrollments.find(
					(entry) => entry.enrollment_id === params.enrollment,
				);
				if (!row || row.state !== "pending") throw notFound("Enrollment");
				row.state = "cancelled";
			},
		},
		{
			method: "GET",
			template: "devices/:id",
			handle: ({ params }) => hub.view(hub.readable(params.id)),
		},
		{
			method: "PATCH",
			template: "devices/:id",
			added: true,
			handle: (request) => {
				const name = displayName(request.body);
				const row = hub.owned(request.params.id);
				row.display_name = name;
				return hub.view(row);
			},
		},
		{
			method: "DELETE",
			template: "devices/:id",
			handle: ({ params }) => {
				const row = hub.owned(params.id);
				row.status = "revoked";
				row.auth_epoch += 1;
				if (hub.version === "current") row.revoked_at = hub.now();
			},
		},
		{
			method: "GET",
			template: "devices/:id/identity",
			handle: ({ params }) => hub.receipt(params.id),
		},
	];
}

function createEnrollment(
	hub: FakeHub,
	request: {
		name?: string;
		api_base_url?: string;
		bootstrap_key?: Ed25519PublicKey;
		controller_key?: Ed25519PublicKey;
		owner_invitation_key?: Ed25519PublicKey;
	},
) {
	const { name, bootstrap_key, controller_key, owner_invitation_key } = request;
	if (!name || !bootstrap_key || !controller_key || !owner_invitation_key)
		throw badRequest("Invalid enrollment request");
	const now = hub.now();
	const open = hub.enrollments.filter(
		(row) => row.state === "pending" && row.expires_at > now,
	);
	if (open.length >= hub.limits.max_pending_enrollments)
		throw hubError(
			429,
			"TOO_MANY_REQUESTS",
			"Pending device enrollment limit reached",
		);
	const manifest: OnboardingManifest = {
		version: 1,
		enrollment_id: crypto.randomUUID(),
		device_id: crypto.randomUUID(),
		owner_id: hub.me,
		name,
		api_base_url: request.api_base_url ?? hub.apiBase,
		bootstrap_key,
		controller_key,
		owner_invitation_key,
		issued_at: now,
		expires_at: now + hub.limits.enrollment_ttl_seconds,
	};
	hub.manifests.set(manifest.device_id, manifest);
	hub.enrollments.push({
		enrollment_id: manifest.enrollment_id,
		device_id: manifest.device_id,
		name,
		state: "pending",
		created_at: now,
		expires_at: manifest.expires_at,
		controller_key_thumbprint: thumbprint(controller_key),
	});
	return {
		enrollment_token: fakeCompact({
			enrollment_id: manifest.enrollment_id,
			expires_at: manifest.expires_at,
		}),
		manifest,
	};
}

function accessRoutes(hub: FakeHub, world: World): Route[] {
	return [
		{
			method: "POST",
			template: "devices/:id/signaling/controller",
			handle: (request) => world.admit(request.params.id, body(request)),
		},
		{
			method: "GET",
			template: "devices/:id/management/policy",
			handle: ({ params }) => {
				hub.visible(params.id);
				return hub.policyView(params.id);
			},
		},
		{
			method: "PUT",
			template: "devices/:id/management/policy",
			handle: (request) =>
				hub.putPolicy(
					request.params.id,
					body<{ policy_jws?: string }>(request).policy_jws ?? "",
				),
		},
		{
			method: "GET",
			template: "devices/:id/management/my-access",
			added: true,
			handle: ({ params }) => hub.myAccess(params.id),
		},
		{
			method: "GET",
			template: "devices/controller-vaults",
			added: true,
			handle: () => hub.backupList(),
		},
		{
			method: "GET",
			template: "devices/controller-vaults/:id",
			handle: ({ params }) => {
				const backup = hub.backups.get(params.id);
				if (!backup) throw notFound("Account backup");
				return {
					public_key: backup.public_key,
					ciphertext: backup.ciphertext,
					revision: backup.revision,
				};
			},
		},
		{
			method: "PUT",
			template: "devices/controller-vaults/:id",
			handle: (request) => hub.putBackup(request.params.id, body(request)),
		},
	];
}

function verified<T>(read: () => T): T {
	try {
		return read();
	} catch {
		throw badRequest("The signature does not verify");
	}
}

function statusRoutes(hub: FakeHub): Route[] {
	return [
		{
			method: "GET",
			template: "devices/:id/fleet/readers/:key",
			handle: ({ params }) => {
				hub.visible(params.id);
				const reader = hub.reader(params.id, params.key);
				if (!reader) throw notFound("Fleet reader");
				return reader;
			},
		},
		{
			method: "PUT",
			template: "devices/:id/fleet/readers/:key",
			handle: (request) => {
				const { id, key } = request.params;
				hub.visible(id);
				const jws = body<{ reader_jws?: string }>(request).reader_jws ?? "";
				return verified(() => hub.putReader(id, key, jws));
			},
		},
		{
			method: "DELETE",
			template: "devices/:id/fleet/readers/:key",
			handle: (request) => {
				const reader = hub.reader(request.params.id, request.params.key);
				if (!reader) throw notFound("Fleet reader");
				reader.deleted = true;
				reader.revision =
					body<{ revision?: number }>(request).revision ?? reader.revision + 1;
			},
		},
		{
			method: "GET",
			template: "devices/:id/fleet/snapshots/:key",
			handle: ({ params }) => hub.fleetView(params.id, params.key),
		},
		{
			method: "GET",
			template: "devices/:id/inventory/:key",
			handle: ({ params }) => hub.inventoryView(params.id, params.key),
		},
		{
			method: "PUT",
			template: "devices/:id/inventory/:key",
			handle: (request) =>
				hub.putInventory(request.params.id, request.params.key, body(request)),
		},
		{
			method: "GET",
			template: "devices/:id/archives",
			handle: ({ params, query }) => {
				hub.visible(params.id);
				const after = Number(query.get("after") ?? 0);
				const rows = (hub.archives.get(params.id) ?? [])
					.filter(
						(row) =>
							row.scope === query.get("scope") &&
							row.kind === query.get("kind") &&
							row.sequence > after,
					)
					.slice(0, 100);
				return {
					archives: rows.map(
						({ ciphertext: _body, recipient_keys: _keys, ...row }) => row,
					),
					next: rows.at(-1)?.sequence ?? after,
				};
			},
		},
		{
			method: "GET",
			template: "devices/:id/archives/:archive",
			handle: ({ params }) => {
				const row = (hub.archives.get(params.id) ?? []).find(
					(entry) => entry.archive_id === params.archive,
				);
				if (!row) throw notFound("History segment");
				return {
					roster_jws: row.roster_jws,
					manifest_jws: row.manifest_jws,
					ciphertext: row.ciphertext,
					recipient_keys: row.recipient_keys,
				};
			},
		},
		{
			method: "GET",
			template: "devices/archive-usage",
			added: true,
			handle: () =>
				hub.archiveUsage ?? {
					tier: "FREE",
					max_bytes: 0,
					retention_seconds: 0,
					used_bytes: 0,
					devices: [],
				},
		},
	];
}

function certificateRoutes(hub: FakeHub): Route[] {
	return [
		{
			method: "GET",
			template: "devices/certificate-inventory",
			added: true,
			handle: () => hub.fleetCertificates(),
		},
		{
			method: "GET",
			template: "devices/:id/certificate-inventory",
			handle: ({ params }) => {
				hub.visible(params.id);
				return (
					hub.certificates.get(params.id) ?? {
						revision: 0,
						updated_at: null,
						certificates: [],
					}
				);
			},
		},
		{
			method: "GET",
			template: "devices/:id/certificate-notices",
			added: true,
			handle: ({ params, query }) =>
				hub.noticeList(
					params.id,
					query.get("certificate"),
					Number(query.get("limit") ?? 50),
				),
		},
		{
			method: "GET",
			template: "devices/:id/certificate-notices/mute",
			added: true,
			handle: ({ params }) => hub.muteList(params.id),
		},
		{
			method: "PUT",
			template: "devices/:id/certificate-notices/mute",
			added: true,
			handle: (request) => {
				const mute = body<{
					certificate_id?: string | null;
					until?: number | null;
				}>(request);
				return hub.mute(
					request.params.id,
					mute.certificate_id ?? null,
					mute.until ?? null,
				);
			},
		},
		{
			method: "DELETE",
			template: "devices/:id/certificate-notices/mute",
			added: true,
			handle: ({ params, query }) => {
				const certificate = query.get("certificate");
				hub.unmute(params.id, certificate === "*" ? null : certificate);
			},
		},
		{
			method: "POST",
			template: "devices/:id/certificate-notices/test",
			added: true,
			handle: (request) =>
				hub.testNotice(
					request.params.id,
					body<{ channel?: string }>(request).channel ?? "both",
				),
		},
	];
}

function newGrant(
	hub: FakeHub,
	deviceId: string,
	request: Partial<ResourceGrant>,
): ResourceGrant {
	const { placement_id, deployment_id, project_id } = request;
	if (!placement_id || !deployment_id || !project_id)
		throw badRequest("Invalid cloud approval request");
	const grant = {
		grant_id: crypto.randomUUID(),
		device_id: deviceId,
		placement_id,
		deployment_id,
		project_id,
		app_id: request.app_id ?? null,
		delegating_user_id: hub.me,
		authz_version: 1,
		model_ids: request.model_ids ?? [],
		online_access: request.online_access ?? null,
		max_instances: request.max_instances ?? 1,
		expires_at: request.expires_at ?? hub.now() + 30 * DAY,
		status: "active",
	} satisfies ResourceGrant;
	hub.resourcesOf(deviceId).grants.push(grant);
	return grant;
}

function approveBilling(
	hub: FakeHub,
	deviceId: string,
	grantId: string,
	request: { limit_micros?: number; expires_at?: number },
): BillingGrant {
	const grant = hub.grant(deviceId, grantId);
	const entry = hub.resourcesOf(deviceId);
	for (const row of entry.billing)
		if (row.grant_id === grantId && row.payer_id === hub.me)
			row.status = "revoked";
	const billing = {
		billing_grant_id: crypto.randomUUID(),
		grant_id: grantId,
		payer_id: hub.me,
		authz_version: 1,
		limit_micros: request.limit_micros ?? 1_000_000,
		used_micros: 0,
		reserved_micros: 0,
		expires_at: request.expires_at ?? grant.expires_at,
		status: "active",
	} satisfies BillingGrant;
	entry.billing.push(billing);
	return billing;
}

function cloudRoutes(hub: FakeHub): Route[] {
	return [
		{
			method: "GET",
			template: "devices/resource-summary",
			added: true,
			handle: () => hub.resourceSummary(),
		},
		{
			method: "GET",
			template: "devices/:id/resource-grants",
			handle: ({ params }) =>
				hub.resourcesOf(params.id).grants.map((grant) => hub.grantView(grant)),
		},
		{
			method: "POST",
			template: "devices/:id/resource-grants",
			handle: (request) =>
				hub.grantView(newGrant(hub, request.params.id, body(request))),
		},
		{
			method: "GET",
			template: "devices/:id/resource-grants/:grant",
			handle: ({ params }) => hub.grantView(hub.grant(params.id, params.grant)),
		},
		{
			method: "DELETE",
			template: "devices/:id/resource-grants/:grant",
			handle: ({ params }) => {
				hub.grant(params.id, params.grant).status = "revoked";
			},
		},
		{
			method: "GET",
			template: "devices/:id/resource-grants/:grant/billing",
			handle: ({ params }) => {
				hub.grant(params.id, params.grant);
				return hub
					.resourcesOf(params.id)
					.billing.filter((row) => row.grant_id === params.grant);
			},
		},
		{
			method: "POST",
			template: "devices/:id/resource-grants/:grant/billing",
			handle: (request) =>
				approveBilling(
					hub,
					request.params.id,
					request.params.grant,
					body(request),
				),
		},
		{
			method: "GET",
			template: "devices/:id/resource-grants/:grant/billing/eligibility",
			added: true,
			handle: ({ params }) => hub.billingEligibility(params.id, params.grant),
		},
		{
			method: "GET",
			template: "devices/:id/billing-grants",
			handle: ({ params }) => hub.resourcesOf(params.id).billing,
		},
		{
			method: "GET",
			template: "devices/:id/billing-grants/:billing",
			handle: ({ params }) => hub.billing(params.id, params.billing),
		},
		{
			method: "DELETE",
			template: "devices/:id/billing-grants/:billing",
			handle: ({ params }) => {
				hub.billing(params.id, params.billing).status = "revoked";
			},
		},
		{
			method: "GET",
			template: "devices/:id/billing-grants/:billing/usage",
			added: true,
			handle: ({ params }) => hub.billingUsage(params.id, params.billing),
		},
		{
			method: "GET",
			template: "devices/:id/instances",
			handle: ({ params }) => hub.resourcesOf(params.id).instances,
		},
		{
			method: "DELETE",
			template: "devices/:id/instances/:instance",
			handle: ({ params }) => {
				const entry = hub.resourcesOf(params.id);
				entry.instances = entry.instances.filter(
					(lease) => lease.instance_id !== params.instance,
				);
			},
		},
		{
			method: "GET",
			template: "apps/:app/device-placements",
			added: true,
			handle: ({ params }) => hub.appPlacements(params.app),
		},
		{
			method: "GET",
			template: "apps/:app/device-metadata",
			handle: ({ params }) => hub.deviceMetadata(params.app),
		},
	];
}

/** Routes only a device's own credential may call; a person's session gets 401. */
const DEVICE_ROUTES: readonly [HubMethod, string][] = [
	["POST", "devices/enrollments/:enrollment/challenge"],
	["POST", "devices/enrollments/:enrollment/redeem"],
	["POST", "devices/token"],
	["POST", "devices/:id/heartbeat"],
	["POST", "devices/:id/receipt"],
	["POST", "devices/:id/signaling/device"],
	["POST", "devices/:id/management/policies"],
	["POST", "devices/:id/management/applied"],
	["POST", "devices/:id/fleet/recipients"],
	["POST", "devices/:id/fleet/snapshots"],
	["POST", "devices/:id/archives"],
	["PUT", "devices/:id/certificate-inventory"],
	["POST", "devices/:id/instances"],
];

function deviceRoutes(): Route[] {
	return DEVICE_ROUTES.map(([method, template]) => ({
		method,
		template,
		handle: () => {
			throw hubError(
				401,
				"UNAUTHORIZED",
				"This route needs a device credential",
			);
		},
	}));
}

/** Every route template the fake hub serves, as `METHOD template`; `added` marks the routes of plan §3.2. */
export interface FakeRouteInfo {
	method: HubMethod;
	template: string;
	added: boolean;
}

function matchTemplate(
	template: string,
	segments: string[],
): Record<string, string> | undefined {
	const parts = template.split("/");
	if (parts.length !== segments.length) return undefined;
	const params: Record<string, string> = {};
	for (const [index, part] of parts.entries()) {
		const segment = segments[index] as string;
		if (part.startsWith(":")) params[part.slice(1)] = segment;
		else if (part !== segment) return undefined;
	}
	return params;
}

const specificity = (template: string) =>
	template.split("/").filter((part) => !part.startsWith(":")).length;

/* Requests, sessions and the relay. */

interface Admission {
	participant: string;
	deviceId: string;
	expiresAt: number;
}

interface Envelope {
	kind: "hello" | "handshake" | "message";
	session_id: string;
	data: string;
}

class World {
	readonly hub: FakeHub;
	readonly calls: FakeCall[] = [];
	readonly commands: FakeCommand[] = [];
	readonly mode: HubMode = {
		reachable: true,
		signedIn: true,
		tokenRestricted: false,
		devicesEnabled: true,
		signaling: true,
	};
	readonly admissions = new Map<string, Admission>();
	private readonly agents = new Map<string, FakeAgent>();
	private readonly routes: Route[];
	private readonly extra: Route[] = [];
	private readonly faults: {
		match: RouteMatch;
		error: unknown;
		remaining: number;
	}[] = [];
	private readonly holds: { match: RouteMatch; gate: Promise<void> }[] = [];

	constructor(
		readonly seed: DeviceSeed,
		private readonly options: FakeDeviceApiOptions,
	) {
		this.hub = new FakeHub(seed, options);
		this.routes = [
			...registryRoutes(this.hub),
			...accessRoutes(this.hub, this),
			...statusRoutes(this.hub),
			...certificateRoutes(this.hub),
			...cloudRoutes(this.hub),
			...deviceRoutes(),
		];
	}

	routeList(): FakeRouteInfo[] {
		return this.routes.map(({ method, template, added }) => ({
			method,
			template,
			added: added === true,
		}));
	}

	/* Agents. */

	private featuresFor(deviceId: string): AgentFeatures | undefined {
		const option = this.options.agentFeatures;
		if (!option) return undefined;
		const perDevice = Object.values(option).some(
			(value) => typeof value === "object",
		);
		return perDevice
			? (option as Record<string, AgentFeatures>)[deviceId]
			: (option as AgentFeatures);
	}

	/** Uploads the seed's tray still tracks: the device holds them, half received. */
	private seededTransfers(deviceId: string): FakeTransfer[] {
		return this.seed.activity.flatMap((item) => {
			const handle = item.resume;
			if (item.target.deviceId !== deviceId || handle?.type !== "transfer")
				return [];
			return [
				{
					transfer_id: handle.transferId,
					descriptor: {
						project_id: handle.projectId,
						manifest_sha256: handle.manifestSha256,
					},
					state: "receiving" as const,
					expires_at: handle.expiresAt,
					manifest_ready: true,
					file_index: null,
					offset: 0,
					complete: false,
					project_path: null,
				},
			];
		});
	}

	agent(deviceId: string): FakeAgent {
		let agent = this.agents.get(deviceId);
		if (!agent) {
			const live = this.seed.live[deviceId];
			const status = this.seed.fleet[deviceId]?.status;
			const inspection = live?.inspection?.value;
			agent = new FakeAgent(
				deviceId,
				this,
				{
					inspection,
					placements:
						inspection?.placements ??
						status?.observations.flatMap((row) => row.placements) ??
						[],
					bootId:
						inspection?.boot_id ??
						status?.bootId ??
						base64url(fakeBytes(`boot:${deviceId}`)).slice(0, 16),
					live,
					transfers: this.seededTransfers(deviceId),
				},
				this.featuresFor(deviceId),
			);
			this.agents.set(deviceId, agent);
		}
		return agent;
	}

	admit(deviceId: string, request: { participant_id?: string }) {
		const row = this.hub.visible(deviceId);
		if (!this.mode.signaling)
			throw hubError(
				503,
				"SERVICE_UNAVAILABLE",
				"Device signaling is not configured",
			);
		const now = this.hub.now();
		const ended =
			row.owner_id !== this.hub.me &&
			row.access_expires_at != null &&
			row.access_expires_at <= now;
		if (row.status !== "active" || ended)
			throw forbidden("Your access to this device has ended");
		// The transport checks the admission against this computer's clock.
		const expiresAt = Math.floor(Date.now() / 1000) + 300;
		const participant = request.participant_id ?? "";
		const token = fakeCompact({ participant, device_id: deviceId, expiresAt });
		this.admissions.set(token, { participant, deviceId, expiresAt });
		const policy = this.hub.policies.get(deviceId);
		return {
			token,
			expires_at: expiresAt,
			device_auth_epoch: row.auth_epoch,
			signaling_urls: [`wss://${new URL(this.hub.origin).host}/ws/devices`],
			ice_servers: [],
			ice_expires_at: null,
			policy_version: policy?.version ?? 0,
			policy_digest: policy?.digest ?? null,
		} satisfies SignalingAdmission;
	}

	/* Requests. */

	on(route: Route): () => void {
		this.extra.unshift(route);
		return () => {
			const index = this.extra.indexOf(route);
			if (index >= 0) this.extra.splice(index, 1);
		};
	}

	fail(match: RouteMatch, error: unknown, times: number): () => void {
		const fault = { match, error, remaining: times };
		this.faults.push(fault);
		return () => {
			fault.remaining = 0;
		};
	}

	hold(match: RouteMatch): () => void {
		let release!: () => void;
		const hold = {
			match,
			gate: new Promise<void>((resolve) => {
				release = resolve;
			}),
		};
		this.holds.push(hold);
		return () => {
			this.holds.splice(this.holds.indexOf(hold), 1);
			release();
		};
	}

	private matches(match: RouteMatch, method: string, path: string) {
		if (match.method && match.method !== method) return false;
		return typeof match.path === "string"
			? match.path.replace(/^\/+/, "") === path.split("?")[0]
			: match.path.test(path);
	}

	private fault(method: string, path: string): unknown {
		const fault = this.faults.find(
			(entry) => entry.remaining > 0 && this.matches(entry.match, method, path),
		);
		if (!fault) return undefined;
		fault.remaining -= 1;
		return fault.error;
	}

	/** The conditions that answer before any route does. */
	private refusal(path: string): unknown {
		if (!this.mode.signedIn)
			return hubError(401, "UNAUTHORIZED", "Sign in to continue");
		if (!path.startsWith("devices")) return undefined;
		if (this.mode.tokenRestricted)
			return forbidden(
				"Device registry access requires an unrestricted personal access token",
			);
		return this.mode.devicesEnabled
			? undefined
			: hubError(
					503,
					"SERVICE_UNAVAILABLE",
					"Standalone device enrollment is not enabled",
				);
	}

	private dispatch(method: HubMethod, path: string, payload: unknown) {
		const [pathname = "", search = ""] = path.split("?");
		const segments = pathname
			.split("/")
			.filter(Boolean)
			.map(decodeURIComponent);
		const active = [...this.extra, ...this.routes].filter(
			(route) => this.hub.version === "current" || !route.added,
		);
		const matched = active
			.map((route) => ({
				route,
				params: matchTemplate(route.template, segments),
			}))
			.filter(
				(entry): entry is { route: Route; params: Record<string, string> } =>
					entry.params !== undefined,
			);
		// Like the hub's router: the most specific path wins, then its methods decide.
		const best = Math.max(
			...matched.map((entry) => specificity(entry.route.template)),
		);
		const candidates = matched.filter(
			(entry) => specificity(entry.route.template) === best,
		);
		if (!candidates.length) throw hubError(404, undefined, "Not Found");
		const hit = candidates.find((entry) => entry.route.method === method);
		if (!hit) throw hubError(405, undefined, "Method Not Allowed");
		return hit.route.handle({
			params: hit.params,
			query: new URLSearchParams(search),
			body: payload,
		});
	}

	async request(
		method: HubMethod,
		rawPath: string,
		payload?: unknown,
		signal?: AbortSignal | null,
	): Promise<unknown> {
		const path = rawPath.replace(/^\/+/, "");
		this.calls.push(
			payload === undefined ? [method, path] : [method, path, redact(payload)],
		);
		await Promise.resolve();
		for (const hold of this.holds.filter((entry) =>
			this.matches(entry.match, method, path),
		))
			await hold.gate;
		signal?.throwIfAborted();
		if (!this.mode.reachable) throw new TypeError("Failed to fetch");
		const failure = this.fault(method, path) ?? this.refusal(path);
		if (failure) throw failure;
		const result = this.dispatch(method, path, payload);
		return result === undefined ? undefined : structuredClone(result);
	}

	/* The relay: admits a controller, then carries its session to the device's agent. */

	session(
		socket: RelaySocket,
		admission: Admission,
		envelope: Envelope,
	): Envelope | undefined | Promise<Envelope | undefined> {
		const agent = this.agent(admission.deviceId);
		if (!agent.reachable) {
			socket.remoteClose();
			return undefined;
		}
		const reply = (kind: Envelope["kind"], data: Uint8Array): Envelope => ({
			kind,
			session_id: envelope.session_id,
			data: base64url(data),
		});
		if (envelope.kind === "hello") {
			agent.attach(socket);
			return reply("handshake", fakeBytes(`noise:${envelope.session_id}`));
		}
		if (envelope.kind === "handshake")
			return reply(
				"message",
				utf8(
					JSON.stringify({
						ready: true,
						device_id: admission.deviceId,
						expires_at: admission.expiresAt,
						boot_id: agent.bootId,
					}),
				),
			);
		const request = JSON.parse(
			decoder.decode(unbase64url(envelope.data, 32_768)),
		) as { operation_id: string; command: Record<string, unknown> };
		return agent
			.receive(socket, request)
			.then((answer) =>
				answer ? reply("message", utf8(JSON.stringify(answer))) : undefined,
			);
	}
}

interface RelaySocket {
	remoteClose(): void;
}

type SocketEvent = { data: string };

function relaySocketClass(world: World): typeof WebSocket {
	class FakeRelaySocket implements RelaySocket {
		static readonly CONNECTING = 0;
		static readonly OPEN = 1;
		static readonly CLOSING = 2;
		static readonly CLOSED = 3;
		readyState = 0;
		protocol = "";
		bufferedAmount = 0;
		onopen: (() => void) | null = null;
		onclose: (() => void) | null = null;
		onerror: (() => void) | null = null;
		onmessage: ((event: SocketEvent) => void) | null = null;
		private readonly admission?: Admission;

		constructor(
			readonly url: string,
			protocols: string | string[] = [],
		) {
			const prefix = "flowlike.jwt.";
			const token = [protocols]
				.flat()
				.find((protocol) => protocol.startsWith(prefix))
				?.slice(prefix.length);
			this.admission = token ? world.admissions.get(token) : undefined;
			setTimeout(() => this.open(), 0);
		}

		private open() {
			if (this.readyState !== 0) return;
			const admission = this.admission;
			if (!admission || !world.mode.reachable) {
				this.remoteClose();
				return;
			}
			this.readyState = 1;
			this.protocol = RELAY_PROTOCOL;
			this.onopen?.();
			this.deliver({
				type: "ready",
				participant_id: admission.participant,
				role: "controller",
				expires_at: admission.expiresAt,
			});
		}

		private deliver(frame: Record<string, unknown>) {
			if (this.readyState === 1)
				this.onmessage?.({ data: JSON.stringify(frame) });
		}

		private forward(admission: Admission, envelope: Envelope) {
			Promise.resolve(world.session(this, admission, envelope)).then(
				(answer) => {
					if (!answer) return;
					this.deliver({
						type: "frame",
						to: admission.participant,
						from: admission.deviceId,
						from_role: "device",
						channel: "noise",
						payload: base64url(utf8(JSON.stringify(answer))),
					});
				},
				() => this.remoteClose(),
			);
		}

		send(data: string) {
			const admission = this.admission;
			if (this.readyState !== 1 || !admission) return;
			const frame = JSON.parse(data) as Record<string, unknown>;
			if (frame.type === "ping") {
				this.deliver({ type: "pong" });
				return;
			}
			if (frame.type !== "frame" || frame.channel !== "noise") return;
			this.forward(
				admission,
				JSON.parse(
					decoder.decode(unbase64url(String(frame.payload), 49_152)),
				) as Envelope,
			);
		}

		close() {
			this.end();
		}

		remoteClose() {
			if (this.end()) this.onclose?.();
		}

		private end(): boolean {
			if (this.readyState === 3) return false;
			this.readyState = 3;
			if (this.admission) world.agent(this.admission.deviceId).detach(this);
			return true;
		}
	}
	return FakeRelaySocket as unknown as typeof WebSocket;
}

const urlOf = (input: RequestInfo | URL) =>
	typeof input === "string" || input instanceof URL ? String(input) : input.url;

function hubFetchOf(world: World): typeof fetch {
	const answer = async (input: RequestInfo | URL): Promise<Response> => {
		const url = urlOf(input);
		const hubRecord = url === `${world.hub.origin}/api/v1`;
		world.calls.push(["GET", hubRecord ? "/api/v1" : url]);
		await Promise.resolve();
		if (!world.mode.reachable) throw new TypeError("Failed to fetch");
		if (hubRecord)
			return new Response(JSON.stringify(world.hub.record(world.mode)), {
				status: 200,
				headers: { "content-type": "application/json" },
			});
		const release = world.hub.release;
		return release?.url === url
			? new Response(release.jws, { status: 200 })
			: new Response("Not Found", { status: 404 });
	};
	return answer as typeof fetch;
}

function requestBody(options?: RequestInit): unknown {
	const payload = options?.body;
	if (typeof payload !== "string") return payload ?? undefined;
	try {
		return JSON.parse(payload);
	} catch {
		return payload;
	}
}

const NON_WRITES = /\/signaling\/controller$/;

export function fakeDeviceApi(
	options: FakeDeviceApiOptions = {},
): FakeDeviceApi {
	const seed = options.seed ?? sampleFleet();
	const world = new World(seed, options);
	const { hub } = world;
	const send = <T>(method: HubMethod, path: string, data?: unknown) =>
		world.request(method, path, data) as Promise<T>;
	const sent = (method?: HubMethod, path?: string | RegExp) =>
		world.calls.filter(
			([callMethod, callPath]) =>
				(!method || callMethod === method) &&
				(path === undefined ||
					(typeof path === "string"
						? callPath.split("?")[0] === path.replace(/^\/+/, "")
						: path.test(callPath))),
		);
	return {
		seed,
		hub,
		scope: {
			issuer: hub.issuer,
			account: hub.me,
			apiOrigin: hub.origin,
			profileId: options.profileId ?? "profile-1",
		},
		profile: {
			id: options.profileId ?? "profile-1",
			name: "Test profile",
			hub: hub.origin,
			bits: [],
			created: "2026-01-01T00:00:00Z",
			updated: "2026-01-01T00:00:00Z",
		},
		calls: world.calls,
		commands: world.commands,
		mode: world.mode,
		agent: (deviceId) => world.agent(deviceId),
		on: (method, template, handle) => world.on({ method, template, handle }),
		fail: (
			match,
			error = hubError(503, "SERVICE_UNAVAILABLE", "Please retry later"),
			times = Number.POSITIVE_INFINITY,
		) => world.fail(match, error, times),
		hold: (match) => world.hold(match),
		sent,
		writes: () =>
			world.calls.filter(
				([method, path]) => method !== "GET" && !NON_WRITES.test(path),
			),
		hubFetch: hubFetchOf(world),
		WebSocket: relaySocketClass(world),
		fetch: <T>(_profile: IProfile, path: string, init?: RequestInit) =>
			world.request(
				(init?.method ?? "GET").toUpperCase() as HubMethod,
				path,
				requestBody(init),
				init?.signal,
			) as Promise<T>,
		get: <T>(_profile: IProfile, path: string) => send<T>("GET", path),
		post: <T>(_profile: IProfile, path: string, data?: unknown) =>
			send<T>("POST", path, data),
		put: <T>(_profile: IProfile, path: string, data?: unknown) =>
			send<T>("PUT", path, data),
		patch: <T>(_profile: IProfile, path: string, data?: unknown) =>
			send<T>("PATCH", path, data),
		del: <T>(_profile: IProfile, path: string, data?: unknown) =>
			send<T>("DELETE", path, data),
		stream: () =>
			Promise.reject(new Error("The fake device hub has no streaming routes.")),
	};
}

/** The fake hub's route table: `added` marks the routes an old hub lacks. */
export function fakeHubRoutes(): FakeRouteInfo[] {
	return new World(sampleFleet(), {}).routeList();
}
