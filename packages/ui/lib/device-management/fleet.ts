import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { isMissingResourceError } from "../api-error";
import {
	inventoryObservation,
	inventoryScopeKey,
	type RetainedObservation,
} from "./inventory";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	updateFleetState,
} from "./storage";
import { digestText } from "./telemetry";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	FleetAudience,
	FleetLocalState,
	FleetReaderState,
	FleetTrustedContext,
	FleetView,
} from "./types";

export interface FleetMetrics {
	scope: FleetAudience["scope"];
	observedAt: number;
	sample: Record<string, unknown>;
}
export interface OpenFleet {
	observations: RetainedObservation[];
	metrics: FleetMetrics[];
}
function path(device: string, key: string, resource: string) {
	return `devices/${encodeURIComponent(device)}/fleet/${resource}/${encodeURIComponent(key)}`;
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
export function fleetTrusted(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
): FleetTrustedContext {
	return {
		api_base_url: `${scope.apiOrigin.replace(/\/$/u, "")}/api/v1`,
		user_id: scope.account,
		onboarding_manifest_jws: vault.manifestJws,
		owner_controller_key:
			vault.ownerControllerKey ?? vault.controllerPublic.controller_key,
	};
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
): Promise<void> {
	const key = controller.publicBundle().controller_key.x;
	const url = path(vault.deviceId, key, "readers");
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
		throw new Error("Fleet reader registration moved backwards.");
	if (local.pending) {
		if (remote?.reader_jws === local.pending.reader_jws && !remote.deleted) {
			const accepted = local.pending;
			local = await updateFleetState(
				account,
				vault.deviceId,
				key,
				(current) => {
					if (
						current.pending?.reader_jws !== accepted.reader_jws ||
						current.revision > accepted.revision
					)
						throw new Error("Fleet registration changed in another session.");
					return {
						...current,
						revision: accepted.revision,
						readerJws: accepted.reader_jws,
						pending: undefined,
					};
				},
			);
		} else if (remote && remote.revision >= local.pending.revision) {
			throw new Error(
				"Fleet registration changed in another session. Reopen device monitoring.",
			);
		}
	}
	if (!local.pending && remote && !remote.deleted) {
		const verified = controller.verifyFleetReader(
			fleetTrusted(account, vault),
			receipt,
			remote.reader_jws,
		);
		if (verified.revision !== remote.revision)
			throw new Error("Fleet reader revision does not match its signature.");
		if (
			remote.revision === local.revision &&
			local.readerJws &&
			remote.reader_jws !== local.readerJws
		)
			throw new Error("Fleet reader changed at the same revision.");
		if (verified.expires_at > now + 7 * 86400) {
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
			BigInt(now + 365 * 86400),
		);
		local = await updateFleetState(account, vault.deviceId, key, (current) => {
			if (current.pending || current.revision !== local.revision)
				throw new Error("Fleet reader changed in another session.");
			return { ...current, pending: { reader_jws, revision } };
		});
	}
	if (!active()) throw new Error("Fleet unlock cancelled.");
	const pending = local.pending!;
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
		throw new Error(
			"A fleet snapshot moved backwards or changed at the same sequence.",
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
): Promise<OpenFleet> {
	const key = controller.publicBundle().controller_key.x;
	const [receipt, view] = await Promise.all([
		api.get<DeviceReceipt>(
			profile,
			`devices/${encodeURIComponent(vault.deviceId)}/identity`,
		),
		api.get<FleetView>(profile, path(vault.deviceId, key, "snapshots")),
	]);
	if (!active()) throw new Error("Fleet unlock cancelled.");
	if (!Array.isArray(view.snapshots) || view.snapshots.length > 128)
		throw new Error("Fleet snapshot count exceeds its bound.");
	const now = Math.floor(Date.now() / 1000);
	const verified = controller.verifyFleetView(
		fleetTrusted(account, vault),
		receipt,
		view,
		BigInt(now),
	);
	const policy = view.policy_jws
		? crypto.verifyManagementPolicy(
				view.policy_jws,
				crypto.verifyDeviceReceipt(
					receipt,
					vault.manifestJws,
					vault.ownerControllerKey ?? vault.controllerPublic.controller_key,
				).owner_invitation_key,
			)
		: undefined;
	const policyDigest = view.policy_jws
		? await digestText(view.policy_jws)
		: undefined;
	const next: FleetLocalState["anchors"] = {};
	const result: OpenFleet = { observations: [], metrics: [] };
	for (const bundle of view.snapshots) {
		if (!active()) throw new Error("Fleet unlock cancelled.");
		const bytes = controller.openFleet(
			fleetTrusted(account, vault),
			receipt,
			view,
			bundle,
			BigInt(now),
		);
		try {
			// The Rust verifier has authenticated this manifest and its entire payload.
			const manifest = decode<{
				sequence: number;
				audience: FleetAudience;
				observed_at: number;
				boot_id: string;
			}>(bundle.manifest_jws);
			const id = JSON.stringify([
				inventoryScopeKey(manifest.audience.scope),
				manifest.audience.kind,
				manifest.audience.grant_id,
			]);
			if (next[id]) throw new Error("Duplicate fleet stream.");
			const value = JSON.parse(
				new TextDecoder("utf-8", { fatal: true }).decode(bytes),
			);
			if (manifest.observed_at > now + 30)
				throw new Error("Fleet snapshot is from the future.");
			if (manifest.audience.kind === "status") {
				if (
					value.inspection.device_id !== vault.deviceId ||
					value.inspection.boot_id !== manifest.boot_id ||
					value.inspection.observed_at !== manifest.observed_at * 1000
				)
					throw new Error("Fleet inspection identity or timestamp changed.");
				const observation = inventoryObservation(
					value.inspection,
					manifest.audience.scope,
				);
				if (
					observation.placements.length !== value.inspection.placements.length
				)
					throw new Error("Fleet inspection exceeds its granted scope.");
				result.observations.push({
					...observation,
					scope: manifest.audience.scope,
				});
			} else {
				if (
					!Array.isArray(value.metrics.records) ||
					value.metrics.records.length > 1024
				)
					throw new Error("Invalid fleet metrics.");
				result.metrics.push({
					scope: manifest.audience.scope,
					observedAt: manifest.observed_at,
					sample: value.metrics,
				});
			}
			next[id] = {
				sequence: manifest.sequence,
				digest: await digestText(bundle.manifest_jws),
				scope: manifest.audience.scope,
				kind: manifest.audience.kind,
				grant_id: manifest.audience.grant_id,
				reader_digest: manifest.audience.reader_digest,
				policy_digest: manifest.audience.policy_digest,
			};
		} finally {
			bytes.fill(0);
		}
	}
	if (!active()) throw new Error("Fleet unlock cancelled.");
	await updateFleetState(account, vault.deviceId, key, (current) => {
		if (
			verified.reader_revision < current.revision ||
			(verified.reader_revision === current.revision &&
				current.readerJws &&
				current.readerJws !== view.reader_jws)
		)
			throw new Error("Fleet reader moved backwards.");
		if (
			policy &&
			(policy.policy_version < (current.policyVersion ?? 0) ||
				(policy.policy_version === current.policyVersion &&
					policyDigest !== current.policyDigest))
		)
			throw new Error("Fleet access policy moved backwards.");
		if (!policy && current.policyVersion && vault.grantId !== "owner")
			throw new Error("The current fleet access policy is missing.");
		const changedAuthority =
			verified.reader_revision > current.revision ||
			(policy && policy.policy_version > (current.policyVersion ?? 0));
		const authorized = new Set(
			verified.audiences.map((a) =>
				JSON.stringify([inventoryScopeKey(a.scope), a.kind, a.grant_id]),
			),
		);
		const retained = Object.fromEntries(
			Object.entries(current.anchors).filter(
				([id]) => !changedAuthority || authorized.has(id),
			),
		);
		for (const [id, anchor] of Object.entries(next))
			checkFleetAnchor(current.anchors[id], anchor.sequence, anchor.digest);
		for (const [id, anchor] of Object.entries(current.anchors)) {
			if (
				!next[id] &&
				verified.audiences.some(
					(a) =>
						a.kind === anchor.kind &&
						a.grant_id === anchor.grant_id &&
						a.reader_digest === anchor.reader_digest &&
						a.policy_digest === anchor.policy_digest &&
						inventoryScopeKey(a.scope) === inventoryScopeKey(anchor.scope),
				)
			)
				throw new Error("A previously observed fleet snapshot is missing.");
		}
		return {
			...current,
			revision: verified.reader_revision,
			readerJws: view.reader_jws,
			anchors: { ...retained, ...next },
			policyVersion: policy?.policy_version ?? current.policyVersion,
			policyDigest: policyDigest ?? current.policyDigest,
		};
	});
	return result;
}
