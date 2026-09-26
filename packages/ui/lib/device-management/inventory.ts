import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { placement } from "./inspection";
import { type DeviceAccountScope, accountStorageKey } from "./storage";
import { digestText } from "./telemetry";
import type {
	BrowserController,
	EncryptedInventory,
	Inspection,
	InventoryBinding,
	InventoryScope,
	InventoryView,
	PlacementStatus,
} from "./types";

export function inventoryScopeKey(scope: InventoryScope): string {
	return scope.kind === "device"
		? "device"
		: scope.kind === "project"
			? `project/${scope.project_id}`
			: `placement/${scope.project_id}/${scope.placement_id}`;
}
export function inventoryScopeContains(
	allowed: InventoryScope,
	requested: InventoryScope,
): boolean {
	return (
		allowed.kind === "device" ||
		(requested.kind !== "device" &&
			allowed.project_id === requested.project_id &&
			(allowed.kind === "project" ||
				(requested.kind === "placement" &&
					allowed.placement_id === requested.placement_id)))
	);
}
function includes(scope: InventoryScope, row: PlacementStatus): boolean {
	return (
		scope.kind === "device" ||
		(scope.project_id === row.project_id &&
			(scope.kind === "project" || scope.placement_id === row.id))
	);
}
/** Retain only the public inspection shape. Extra device fields cannot carry secrets into inventory. */
export function inventoryObservation(
	inspection: Inspection,
	scope: InventoryScope,
): Inspection {
	if (
		!Number.isSafeInteger(inspection.observed_at) ||
		typeof inspection.observed_at !== "number" ||
		inspection.observed_at <= 0 ||
		inspection.placements.length > 1024 ||
		!inspection.placements.every(placement)
	)
		throw new Error("Invalid retained observation.");
	return {
		device_id: inspection.device_id,
		boot_id: inspection.boot_id,
		observed_at: inspection.observed_at,
		placements: inspection.placements
			.filter((row) => includes(scope, row))
			.map((row) => ({
				id: row.id,
				project_id: row.project_id,
				deployment_id: row.deployment_id,
				revision: row.revision,
				desired_state: row.desired_state,
				observed_state: row.observed_state,
				config_revision: row.config_revision,
				intent_revision: row.intent_revision,
				applied_revision: row.applied_revision,
				desired_replicas: row.desired_replicas,
				running_replicas: row.running_replicas,
				ready_replicas: row.ready_replicas,
				max_replicas: row.max_replicas,
				replicas: row.replicas?.map((replica) => ({
					slot: replica.slot,
					observed_state: replica.observed_state,
					applied_revision: replica.applied_revision,
				})),
			})),
	};
}
interface Anchor {
	scope: InventoryScope;
	revision: number;
	digest: string;
}
export interface RetainedObservation extends Inspection {
	scope: InventoryScope;
}
export function visibleInventory(
	observations: RetainedObservation[],
	projectId?: string,
): { row: PlacementStatus; at: number }[] {
	const sorted = [...observations].sort(
		(a, b) => (b.observed_at ?? 0) - (a.observed_at ?? 0),
	);
	const result = new Map<string, { row: PlacementStatus; at: number }>();
	for (const [index, observation] of sorted.entries())
		for (const row of observation.placements) {
			if (
				(!projectId || row.project_id === projectId) &&
				!sorted.slice(0, index).some((newer) => includes(newer.scope, row))
			)
				result.set(row.id, { row, at: observation.observed_at ?? 0 });
		}
	return [...result.values()];
}
export function checkInventoryAnchor(
	previous: Anchor | undefined,
	encrypted: EncryptedInventory,
	digest: string,
): void {
	if (
		!Number.isSafeInteger(encrypted.binding.revision) ||
		encrypted.binding.revision < 1 ||
		(previous &&
			(encrypted.binding.revision < previous.revision ||
				(encrypted.binding.revision === previous.revision &&
					digest !== previous.digest)))
	)
		throw new Error(
			"Retained inventory moved backwards or changed at the same revision. Reconnect to verify it.",
		);
}
function anchorKey(
	scope: DeviceAccountScope,
	controller: BrowserController,
): string {
	const publicKey = controller.publicBundle();
	return `flow-like/inventory/${JSON.stringify([accountStorageKey(scope), publicKey.device_id, publicKey.controller_key.x])}`;
}
function path(controller: BrowserController): string {
	const identity = controller.publicBundle();
	return `devices/${encodeURIComponent(identity.device_id)}/inventory/${encodeURIComponent(identity.controller_key.x)}`;
}
export function inventoryPath(deviceId: string, key: string): string {
	return `devices/${encodeURIComponent(deviceId)}/inventory/${encodeURIComponent(key)}`;
}
function binding(
	scope: DeviceAccountScope,
	controller: BrowserController,
	requested: InventoryScope,
	revision: number,
): InventoryBinding {
	const identity = controller.publicBundle();
	return {
		issuer: scope.issuer,
		api_origin: scope.apiOrigin,
		account_id: scope.account,
		device_id: identity.device_id,
		controller_key: identity.controller_key,
		scope: requested,
		revision,
	};
}

function encryptedIdentity(encrypted: EncryptedInventory): string {
	const value = encrypted.binding;
	return JSON.stringify([
		value.issuer,
		value.api_origin,
		value.account_id,
		value.device_id,
		value.controller_key.x,
		inventoryScopeKey(value.scope),
		value.revision,
		encrypted.ciphertext,
	]);
}

export async function openInventoryView(
	scope: DeviceAccountScope,
	controller: BrowserController,
	view: InventoryView,
	active: () => boolean = () => true,
): Promise<RetainedObservation[]> {
	const key = anchorKey(scope, controller);
	const anchors: Record<string, Anchor> = JSON.parse(
		localStorage.getItem(key) ?? "{}",
	);
	const next: Record<string, Anchor> = {};
	const result: RetainedObservation[] = [];
	const seen = new Set<string>();
	for (const encrypted of view.observations) {
		const id = inventoryScopeKey(encrypted.binding.scope);
		if (
			seen.has(id) ||
			!view.scopes.some((allowed) =>
				inventoryScopeContains(allowed, encrypted.binding.scope),
			)
		)
			throw new Error("Retained inventory is outside current access.");
		seen.add(id);
		const digest = await digestText(encryptedIdentity(encrypted));
		if (!active()) throw new Error("Inventory unlock cancelled.");
		checkInventoryAnchor(anchors[id], encrypted, digest);
		const bytes = controller.openInventory(
			binding(
				scope,
				controller,
				encrypted.binding.scope,
				encrypted.binding.revision,
			),
			encrypted,
		);
		try {
			const parsed = JSON.parse(
				new TextDecoder("utf-8", { fatal: true }).decode(bytes),
			) as Inspection;
			if (
				parsed.device_id !== controller.publicBundle().device_id ||
				(parsed.boot_id !== null && typeof parsed.boot_id !== "string") ||
				!Array.isArray(parsed.placements) ||
				parsed.placements.some((row) => !includes(encrypted.binding.scope, row))
			)
				throw new Error("Invalid retained inventory scope.");
			result.push({
				...inventoryObservation(parsed, encrypted.binding.scope),
				scope: encrypted.binding.scope,
			});
		} finally {
			bytes.fill(0);
		}
		next[id] = {
			scope: encrypted.binding.scope,
			revision: encrypted.binding.revision,
			digest,
		};
	}
	// A previously sealed write can finish after its controller is closed and
	// while this reader awaits hashing. Preserve the latest local revision floor.
	const latest: Record<string, Anchor> = JSON.parse(
		localStorage.getItem(key) ?? "{}",
	);
	for (const [id, anchor] of Object.entries(latest)) {
		if (
			view.scopes.some((allowed) =>
				inventoryScopeContains(allowed, anchor.scope),
			) &&
			!seen.has(id)
		)
			throw new Error(
				"Previously retained inventory is missing. Reconnect to verify it.",
			);
	}
	for (const encrypted of view.observations) {
		const id = inventoryScopeKey(encrypted.binding.scope);
		checkInventoryAnchor(latest[id], encrypted, next[id].digest);
	}
	localStorage.setItem(key, JSON.stringify({ ...latest, ...next }));
	return result;
}

export type InventoryWriter = (inspection: Inspection) => Promise<void>;

/** Load revisions while unlocked; each writer call seals before yielding to network I/O. */
export async function createInventoryWriter(
	api: IApiState,
	profile: IProfile,
	account: DeviceAccountScope,
	controller: BrowserController,
	grantId: string,
	active: () => boolean,
): Promise<InventoryWriter> {
	const deviceId = controller.publicBundle().device_id;
	const url = path(controller);
	const key = anchorKey(account, controller);
	const view = await api.get<InventoryView>(
		profile,
		`${url}?grant_id=${encodeURIComponent(grantId)}`,
	);
	if (!active()) throw new Error("Inventory unlock cancelled.");
	const retained = await openInventoryView(account, controller, view, active);
	// Use the broadest current grant once. The payload never includes unobserved plans.
	const scopes = view.scopes.filter(
		(scope, index, all) =>
			!all.some(
				(other, otherIndex) =>
					otherIndex !== index &&
					inventoryScopeContains(other, scope) &&
					(inventoryScopeKey(other) !== inventoryScopeKey(scope) ||
						otherIndex < index),
			),
	);
	const revisions = new Map(
		view.observations.map((row) => [
			inventoryScopeKey(row.binding.scope),
			row.binding.revision,
		]),
	);
	const observed = new Map(
		retained.map((row) => [inventoryScopeKey(row.scope), row.observed_at ?? 0]),
	);
	let queue = Promise.resolve();
	let failed = false;
	return (inspection) => {
		if (!active()) throw new Error("Inventory unlock cancelled.");
		if (failed)
			throw new Error("Reconnect before retaining another inspection.");
		if (inspection.device_id !== deviceId)
			throw new Error("Inventory device mismatch.");
		const writes: EncryptedInventory[] = [];
		for (const scope of scopes) {
			const id = inventoryScopeKey(scope);
			if ((observed.get(id) ?? 0) >= (inspection.observed_at ?? 0)) continue;
			const bytes = new TextEncoder().encode(
				JSON.stringify(inventoryObservation(inspection, scope)),
			);
			try {
				writes.push(
					controller.sealInventory(
						binding(account, controller, scope, (revisions.get(id) ?? 0) + 1),
						bytes,
					),
				);
			} finally {
				bytes.fill(0);
			}
		}
		for (const encrypted of writes) {
			const id = inventoryScopeKey(encrypted.binding.scope);
			revisions.set(id, encrypted.binding.revision);
			observed.set(id, inspection.observed_at ?? 0);
		}
		// Only ciphertext crosses this asynchronous boundary. Controller cleanup
		// cannot cancel an observation that was already accepted from the device.
		queue = queue
			.then(async () => {
				for (const encrypted of writes)
					await commitInventory(api, profile, url, key, encrypted);
			})
			.catch((error: unknown) => {
				failed = true;
				throw error;
			});
		return queue;
	};
}

async function commitInventory(
	api: IApiState,
	profile: IProfile,
	url: string,
	key: string,
	encrypted: EncryptedInventory,
): Promise<void> {
	const view = await api.put<InventoryView>(profile, url, encrypted);
	if (
		!view.scopes.some((allowed) =>
			inventoryScopeContains(allowed, encrypted.binding.scope),
		) ||
		!view.observations.some(
			(row) => encryptedIdentity(row) === encryptedIdentity(encrypted),
		)
	)
		throw new Error(
			"Encrypted inventory acknowledgement changed. Reconnect to verify it.",
		);
	const digest = await digestText(encryptedIdentity(encrypted));
	const anchors: Record<string, Anchor> = JSON.parse(
		localStorage.getItem(key) ?? "{}",
	);
	const id = inventoryScopeKey(encrypted.binding.scope);
	checkInventoryAnchor(anchors[id], encrypted, digest);
	anchors[id] = {
		scope: encrypted.binding.scope,
		revision: encrypted.binding.revision,
		digest,
	};
	localStorage.setItem(key, JSON.stringify(anchors));
}
