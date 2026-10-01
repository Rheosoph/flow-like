import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { isMissingResourceError } from "../api-error";
import {
	type SnapshotDeviceFacts,
	snapshotDeviceFacts,
	snapshotPlacement,
} from "./inspection";
import {
	type RetainedObservation,
	SnapshotIntegrityError,
	integrityError,
	inventoryObservation,
	inventoryScopeKey,
} from "./inventory";
import type { PlacementStatusPlus } from "./model/types";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	deviceApiBase,
	pinDeviceIdentity,
	updateFleetState,
} from "./storage";
import { digestText } from "./telemetry";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	EncryptedFleetSnapshot,
	FleetAudience,
	FleetLocalState,
	FleetReaderState,
	FleetTrustedContext,
	FleetView,
	InventoryScope,
	ManagementGrant,
	ManagementPolicy,
} from "./types";

/** A reader declaration with this little time left is renewed (C6, `status_subscription_expiring`). */
export const FLEET_READER_RENEW_S = 30 * 86400;
const FLEET_READER_LIFETIME_S = 365 * 86400;

export interface FleetMetrics {
	scope: FleetAudience["scope"];
	observedAt: number;
	sample: Record<string, unknown>;
}
export interface OpenFleet {
	observations: RetainedObservation[];
	metrics: FleetMetrics[];
}
/** One signed snapshot stream the hub returned in this read. */
export interface FleetStream {
	/** `fleetStreamId(audience)`. */
	id: string;
	kind: FleetAudience["kind"];
	scope: InventoryScope;
	grantId: string;
	sequence: number;
	bootId: string;
	/** Unix seconds. */
	observedAt: number;
}
/** A status snapshot: the retained shape plus the validated facts newer agents attach (plan §3.4.2). */
export type StatusObservation = RetainedObservation &
	SnapshotDeviceFacts & { placements: PlacementStatusPlus[] };
export interface FleetRead extends OpenFleet {
	observations: StatusObservation[];
	readerRevision: number;
	/** Unix seconds. */
	readerExpiresAt: number;
	/** The newest verified owner policy this computer has seen. */
	policyVersion?: number;
	/** The viewer's own grant in the verified policy; undefined for the owner. */
	myGrant?: ManagementGrant;
	streams: FleetStream[];
	/** Stream ids the verified reader and policy still authorize, including ones without a snapshot yet. */
	authorized: string[];
}
export interface ReadFleetOptions {
	/** The key session's verified and pinned receipt: no identity read, no second pin (D-d). */
	receipt?: DeviceReceipt;
	/** Hub-corrected unix seconds. */
	now?: number;
}

function path(device: string, key: string, resource: string) {
	return `devices/${encodeURIComponent(device)}/fleet/${resource}/${encodeURIComponent(key)}`;
}
export function fleetReaderPath(deviceId: string, controllerKey: string) {
	return path(deviceId, controllerKey, "readers");
}
function decode<T>(compact: string): T {
	const part = compact.split(".")[1];
	if (!part || compact.length > 32_768)
		throw new Error("Invalid signed fleet declaration.");
	const bytes = Uint8Array.from(
		atob(part.replaceAll("-", "+").replaceAll("_", "/")),
		(c) => c.charCodeAt(0),
	);
	return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
}
function verifying<T>(run: () => T): T {
	try {
		return run();
	} catch (error) {
		throw integrityError(error);
	}
}
export function fleetStreamId(audience: FleetAudience): string {
	return JSON.stringify([
		inventoryScopeKey(audience.scope),
		audience.kind,
		audience.grant_id,
	]);
}
export function fleetTrusted(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
): FleetTrustedContext {
	return {
		api_base_url: deviceApiBase(scope),
		user_id: scope.account,
		onboarding_manifest_jws: vault.manifestJws,
		owner_controller_key:
			vault.ownerControllerKey ?? vault.controllerPublic.controller_key,
	};
}

export interface RegisterFleetReaderOptions {
	/** Sign a new 365-day declaration even when more than `FLEET_READER_RENEW_S` is left ("Renew"). */
	renew?: boolean;
}

/** Persist the exact signed request before upload, so a lost acknowledgement is retryable. */
export async function registerFleetReader(
	api: IApiState,
	profile: IProfile,
	account: DeviceAccountScope,
	controller: BrowserController,
	vault: LocalDeviceVault,
	receipt: DeviceReceipt,
	active: () => boolean = () => true,
	options: RegisterFleetReaderOptions = {},
): Promise<void> {
	const key = controller.publicBundle().controller_key.x;
	const url = fleetReaderPath(vault.deviceId, key);
	const now = Math.floor(Date.now() / 1000);
	let local = await updateFleetState(
		account,
		vault.deviceId,
		key,
		(current) => current,
	);
	const remote = await api
		.get<FleetReaderState>(profile, url)
		.catch((error) => {
			if (isMissingResourceError(error)) return undefined;
			throw error;
		});
	if (!active()) throw new Error("Fleet unlock cancelled.");
	if (
		remote &&
		(!Number.isSafeInteger(remote.revision) ||
			remote.revision < local.revision ||
			remote.revision < 1)
	)
		throw new SnapshotIntegrityError(
			"Fleet reader registration moved backwards.",
		);
	const verifyRemote = (state: FleetReaderState) => {
		const verified = verifying(() =>
			controller.verifyFleetReader(
				fleetTrusted(account, vault),
				receipt,
				state.reader_jws,
			),
		);
		if (verified.revision !== state.revision)
			throw new SnapshotIntegrityError(
				`Fleet reader revision ${state.revision} does not match its signed revision ${verified.revision}.`,
			);
		return verified;
	};
	if (local.pending && remote) {
		const pending = local.pending;
		const settled = remote.reader_jws === pending.reader_jws && !remote.deleted;
		if (settled || remote.revision >= pending.revision) {
			// A restored copy of this controller key may have registered this revision first.
			if (!settled && !remote.deleted) verifyRemote(remote);
			const adopted = settled ? pending : remote.deleted ? undefined : remote;
			local = await updateFleetState(
				account,
				vault.deviceId,
				key,
				(current) => {
					if (
						current.pending?.reader_jws !== pending.reader_jws ||
						(adopted && current.revision > adopted.revision)
					)
						throw new Error("Fleet registration changed in another session.");
					return adopted
						? {
								...current,
								revision: adopted.revision,
								readerJws: adopted.reader_jws,
								pending: undefined,
							}
						: { ...current, pending: undefined };
				},
			);
			if (!active()) throw new Error("Fleet unlock cancelled.");
		}
	}
	if (!local.pending && remote && !remote.deleted) {
		const verified = verifyRemote(remote);
		if (
			remote.revision === local.revision &&
			local.readerJws &&
			remote.reader_jws !== local.readerJws
		)
			throw new SnapshotIntegrityError(
				"Fleet reader changed at the same revision.",
			);
		if (!options.renew && verified.expires_at > now + FLEET_READER_RENEW_S) {
			await updateFleetState(account, vault.deviceId, key, (current) => {
				if (
					current.pending ||
					current.revision > remote.revision ||
					(current.revision === remote.revision &&
						current.readerJws &&
						current.readerJws !== remote.reader_jws)
				)
					throw new Error("Fleet reader changed in another session.");
				return {
					...current,
					revision: remote.revision,
					readerJws: remote.reader_jws,
				};
			});
			return;
		}
	}
	if (!local.pending) {
		const revision = Math.max(local.revision, remote?.revision ?? 0) + 1;
		const reader_jws = controller.createFleetReader(
			fleetTrusted(account, vault).api_base_url,
			account.account,
			BigInt(revision),
			BigInt(now),
			BigInt(now + FLEET_READER_LIFETIME_S),
		);
		local = await updateFleetState(account, vault.deviceId, key, (current) => {
			if (current.pending || current.revision !== local.revision)
				throw new Error("Fleet reader changed in another session.");
			return { ...current, pending: { reader_jws, revision } };
		});
	}
	if (!active()) throw new Error("Fleet unlock cancelled.");
	const pending = local.pending;
	if (!pending)
		throw new Error("Fleet registration has no pending declaration to upload.");
	const confirmed = await api.put<FleetReaderState>(profile, url, {
		reader_jws: pending.reader_jws,
	});
	if (
		confirmed.reader_jws !== pending.reader_jws ||
		confirmed.revision !== pending.revision ||
		confirmed.deleted
	)
		throw new Error(
			"Fleet registration acknowledgement changed. Retry the same registration.",
		);
	await updateFleetState(account, vault.deviceId, key, (current) => {
		if (
			current.pending?.reader_jws !== pending.reader_jws ||
			current.revision > pending.revision
		)
			throw new Error("Pending fleet registration changed.");
		return {
			...current,
			revision: pending.revision,
			readerJws: pending.reader_jws,
			pending: undefined,
		};
	});
}

/** "Stop receiving": the hub stops handing this controller's snapshots out; local rollback floors stay. */
export async function removeFleetReader(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	controllerKey: string,
): Promise<void> {
	const url = fleetReaderPath(deviceId, controllerKey);
	const reader = await api
		.get<FleetReaderState>(profile, url)
		.catch((error) => {
			if (isMissingResourceError(error)) return undefined;
			throw error;
		});
	if (!reader || reader.deleted) return;
	await api.del(profile, url, { revision: reader.revision + 1 });
}

export function checkFleetAnchor(
	previous: FleetLocalState["anchors"][string] | undefined,
	sequence: number,
	digest: string,
): void {
	if (
		!Number.isSafeInteger(sequence) ||
		sequence < 1 ||
		(previous &&
			(sequence < previous.sequence ||
				(sequence === previous.sequence && digest !== previous.digest)))
	)
		throw new SnapshotIntegrityError(
			"A fleet snapshot moved backwards or changed at the same sequence.",
		);
}

interface FleetManifest {
	sequence: number;
	audience: FleetAudience;
	observed_at: number;
	boot_id: string;
}
interface OpenContext {
	trusted: FleetTrustedContext;
	controller: BrowserController;
	receipt: DeviceReceipt;
	view: FleetView;
	deviceId: string;
	now: number;
}
interface OpenedStream {
	stream: FleetStream;
	anchor: FleetLocalState["anchors"][string];
	observation?: StatusObservation;
	metric?: FleetMetrics;
}
interface StatusPayload {
	inspection: RetainedObservation & { placements: unknown[] };
}
interface MetricsPayload {
	metrics: Record<string, unknown> & { records?: unknown };
}

function statusObservation(
	value: StatusPayload,
	manifest: FleetManifest,
	deviceId: string,
): StatusObservation {
	if (
		value.inspection.device_id !== deviceId ||
		value.inspection.boot_id !== manifest.boot_id ||
		value.inspection.observed_at !== manifest.observed_at * 1000
	)
		throw new SnapshotIntegrityError(
			"Fleet inspection identity or timestamp changed.",
		);
	const observation = inventoryObservation(
		value.inspection,
		manifest.audience.scope,
	);
	if (observation.placements.length !== value.inspection.placements.length)
		throw new SnapshotIntegrityError(
			"Fleet inspection exceeds its granted scope.",
		);
	// No row was filtered out, so the retained rows line up with the snapshot's.
	return {
		...observation,
		...snapshotDeviceFacts(value.inspection),
		scope: manifest.audience.scope,
		placements: observation.placements.map((row, index) =>
			snapshotPlacement(row, value.inspection.placements[index]),
		),
	};
}
function metricsSample(
	value: MetricsPayload,
	manifest: FleetManifest,
): FleetMetrics {
	if (
		!Array.isArray(value.metrics.records) ||
		value.metrics.records.length > 1024
	)
		throw new SnapshotIntegrityError("Invalid fleet metrics.");
	return {
		scope: manifest.audience.scope,
		observedAt: manifest.observed_at,
		sample: value.metrics,
	};
}

async function openStream(
	context: OpenContext,
	bundle: EncryptedFleetSnapshot,
): Promise<OpenedStream> {
	const bytes = verifying(() =>
		context.controller.openFleet(
			context.trusted,
			context.receipt,
			context.view,
			bundle,
			BigInt(context.now),
		),
	);
	try {
		// The Rust verifier has authenticated this manifest and its entire payload.
		const manifest = decode<FleetManifest>(bundle.manifest_jws);
		const value = JSON.parse(
			new TextDecoder("utf-8", { fatal: true }).decode(bytes),
		);
		if (manifest.observed_at > context.now + 30)
			throw new SnapshotIntegrityError("Fleet snapshot is from the future.");
		const { audience } = manifest;
		const stream: FleetStream = {
			id: fleetStreamId(audience),
			kind: audience.kind,
			scope: audience.scope,
			grantId: audience.grant_id,
			sequence: manifest.sequence,
			bootId: manifest.boot_id,
			observedAt: manifest.observed_at,
		};
		const anchor = {
			sequence: manifest.sequence,
			digest: await digestText(bundle.manifest_jws),
			scope: audience.scope,
			kind: audience.kind,
			grant_id: audience.grant_id,
			reader_digest: audience.reader_digest,
			policy_digest: audience.policy_digest ?? null,
		};
		return audience.kind === "status"
			? {
					stream,
					anchor,
					observation: statusObservation(value, manifest, context.deviceId),
				}
			: { stream, anchor, metric: metricsSample(value, manifest) };
	} catch (error) {
		throw integrityError(error, "Fleet snapshot failed verification.");
	} finally {
		bytes.fill(0);
	}
}

interface Verified {
	reader_revision: number;
	audiences: FleetAudience[];
}
interface FleetUpdate {
	verified: Verified;
	readerJws: string;
	policy?: ManagementPolicy;
	policyDigest?: string;
	grantId: string;
	next: FleetLocalState["anchors"];
}

function checkReader(current: FleetLocalState, update: FleetUpdate) {
	const revision = update.verified.reader_revision;
	const substituted =
		revision === current.revision &&
		Boolean(current.readerJws) &&
		current.readerJws !== update.readerJws;
	if (revision < current.revision || substituted)
		throw new SnapshotIntegrityError("Fleet reader moved backwards.");
}

/** A shared reader's verified policy may not disappear once seen; the owner needs none. */
function checkPolicyKept(current: FleetLocalState, grantId: string) {
	if (current.policyVersion && grantId !== "owner")
		throw new SnapshotIntegrityError(
			"The current fleet access policy is missing.",
		);
}

function checkPolicy(current: FleetLocalState, update: FleetUpdate) {
	const { policy } = update;
	if (!policy) return checkPolicyKept(current, update.grantId);
	const known = current.policyVersion ?? 0;
	const substituted =
		policy.policy_version === current.policyVersion &&
		update.policyDigest !== current.policyDigest;
	if (policy.policy_version < known || substituted)
		throw new SnapshotIntegrityError("Fleet access policy moved backwards.");
}

/** Everything that binds a stream's floor to the authority it was observed under. */
function audienceKey(
	audience: Pick<
		FleetAudience,
		"kind" | "grant_id" | "reader_digest" | "policy_digest" | "scope"
	>,
) {
	return JSON.stringify([
		audience.kind,
		audience.grant_id,
		audience.reader_digest,
		audience.policy_digest ?? null,
		inventoryScopeKey(audience.scope),
	]);
}

/** A stream observed under the still-current authority may not silently disappear. */
function checkMissing(current: FleetLocalState, update: FleetUpdate) {
	const authorized = new Set(update.verified.audiences.map(audienceKey));
	for (const [id, anchor] of Object.entries(current.anchors))
		if (!update.next[id] && authorized.has(audienceKey(anchor)))
			throw new SnapshotIntegrityError(
				"A previously observed fleet snapshot is missing.",
			);
}

/** After a reader or policy change, floors of streams it no longer authorizes are dropped. */
function retainedAnchors(current: FleetLocalState, update: FleetUpdate) {
	const { verified, policy } = update;
	const changedAuthority =
		verified.reader_revision > current.revision ||
		(policy?.policy_version ?? 0) > (current.policyVersion ?? 0);
	if (!changedAuthority) return current.anchors;
	const authorized = new Set(verified.audiences.map(fleetStreamId));
	return Object.fromEntries(
		Object.entries(current.anchors).filter(([id]) => authorized.has(id)),
	);
}

function nextFleetState(
	current: FleetLocalState,
	update: FleetUpdate,
): FleetLocalState {
	checkReader(current, update);
	checkPolicy(current, update);
	const { verified, policy, policyDigest, next } = update;
	const retained = retainedAnchors(current, update);
	for (const [id, anchor] of Object.entries(next))
		checkFleetAnchor(current.anchors[id], anchor.sequence, anchor.digest);
	checkMissing(current, update);
	return {
		...current,
		revision: verified.reader_revision,
		readerJws: update.readerJws,
		anchors: { ...retained, ...next },
		policyVersion: policy?.policy_version ?? current.policyVersion,
		policyDigest: policyDigest ?? current.policyDigest,
	};
}

async function pinReceipt(
	account: DeviceAccountScope,
	deviceId: string,
	receipt: DeviceReceipt,
) {
	try {
		await pinDeviceIdentity(account, deviceId, receipt);
	} catch (error) {
		const message = error instanceof Error ? error.message : "";
		throw /differ from|cannot be pinned/u.test(message)
			? integrityError(error)
			: error;
	}
}

function verifiedPolicy(
	crypto: DeviceCrypto,
	view: FleetView,
	receipt: DeviceReceipt,
	vault: LocalDeviceVault,
): ManagementPolicy | undefined {
	const policyJws = view.policy_jws;
	if (!policyJws) return undefined;
	return verifying(() =>
		crypto.verifyManagementPolicy(
			policyJws,
			crypto.verifyDeviceReceipt(
				receipt,
				vault.manifestJws,
				vault.ownerControllerKey ?? vault.controllerPublic.controller_key,
			).owner_invitation_key,
		),
	);
}

export async function readFleet(
	api: IApiState,
	profile: IProfile,
	account: DeviceAccountScope,
	controller: BrowserController,
	vault: LocalDeviceVault,
	crypto: DeviceCrypto,
	active: () => boolean = () => true,
	options: ReadFleetOptions = {},
): Promise<FleetRead> {
	const key = controller.publicBundle().controller_key.x;
	const [receipt, view] = await Promise.all([
		options.receipt ??
			api.get<DeviceReceipt>(
				profile,
				`devices/${encodeURIComponent(vault.deviceId)}/identity`,
			),
		api.get<FleetView>(profile, path(vault.deviceId, key, "snapshots")),
	]);
	if (!active()) throw new Error("Fleet unlock cancelled.");
	if (!Array.isArray(view.snapshots) || view.snapshots.length > 128)
		throw new SnapshotIntegrityError("Fleet snapshot count exceeds its bound.");
	const now = Math.floor(options.now ?? Date.now() / 1000);
	const trusted = fleetTrusted(account, vault);
	const verified = verifying(() =>
		controller.verifyFleetView(trusted, receipt, view, BigInt(now)),
	);
	if (!options.receipt) await pinReceipt(account, vault.deviceId, receipt);
	const policy = verifiedPolicy(crypto, view, receipt, vault);
	const policyDigest = view.policy_jws
		? await digestText(view.policy_jws)
		: undefined;
	const context: OpenContext = {
		trusted,
		controller,
		receipt,
		view,
		deviceId: vault.deviceId,
		now,
	};
	const next: FleetLocalState["anchors"] = {};
	const result: FleetRead = {
		observations: [],
		metrics: [],
		readerRevision: verified.reader_revision,
		readerExpiresAt: verified.reader_expires_at,
		streams: [],
		authorized: verified.audiences.map(fleetStreamId),
	};
	for (const bundle of view.snapshots) {
		if (!active()) throw new Error("Fleet unlock cancelled.");
		const opened = await openStream(context, bundle);
		if (next[opened.stream.id])
			throw new SnapshotIntegrityError("Duplicate fleet stream.");
		next[opened.stream.id] = opened.anchor;
		result.streams.push(opened.stream);
		if (opened.observation) result.observations.push(opened.observation);
		if (opened.metric) result.metrics.push(opened.metric);
	}
	if (!active()) throw new Error("Fleet unlock cancelled.");
	const committed = await updateFleetState(
		account,
		vault.deviceId,
		key,
		(current) =>
			nextFleetState(current, {
				verified,
				readerJws: view.reader_jws,
				policy,
				policyDigest,
				grantId: vault.grantId,
				next,
			}),
	);
	if (committed.policyVersion !== undefined)
		result.policyVersion = committed.policyVersion;
	const myGrant =
		vault.grantId === "owner"
			? undefined
			: policy?.grants.find((grant) => grant.grant_id === vault.grantId);
	if (myGrant) result.myGrant = myGrant;
	return result;
}
