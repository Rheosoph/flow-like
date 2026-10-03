import { relationshipOf } from "../../../../lib/device-management/model/presence";
import type {
	AccessRequestRecord,
	DeviceRow,
	Relationship,
} from "../../../../lib/device-management/model/types";
import type {
	KeySessionSnapshot,
	LiveState,
	LocalSummary,
	LocalVaultSummary,
} from "../../../../lib/device-management/workspace/types";

/** Where one device's keys and its account backup stand, from this computer's point of view. */
export type KeyCategory =
	| "never"
	| "lost"
	| "hub_newer"
	| "pending"
	| "oldpw"
	| "out_of_date"
	| "unchecked"
	| "restorable"
	| "synced"
	| "nokeys";

/** The summary windows above the table. */
export type KeyGroup = "synced" | "behind" | "never" | "restorable" | "nokeys";

export type LocalBackup = LocalSummary["backups"][string];

export interface AccountBackupFact {
	revision: number;
	/** Unix seconds; only the account backup list (BG25) carries it. */
	updatedAt?: number;
}

export interface KeyRow {
	deviceId: string;
	name: string;
	row: DeviceRow;
	relationship: Relationship;
	session: KeySessionSnapshot | undefined;
	/** The live connection on top of an open key session. */
	live?: LiveState;
	vault?: LocalVaultSummary;
	backup?: LocalBackup;
	/** Account backup version on the hub; undefined while it is not known. */
	hubRevision?: number;
	hubSavedAt?: number;
	category: KeyCategory;
}

export type LocalOnlyReason = "revoked" | "request" | "unlisted";

export interface LocalOnlyRow {
	deviceId: string;
	name: string;
	reason: LocalOnlyReason;
	role: LocalVaultSummary["role"];
	/** Unix seconds: revoked at, or the request's creation. */
	since?: number;
	registeredAt?: number;
	/** Unix seconds of the revoked device's last check-in. */
	lastSeenAt?: number;
	ownerId?: string;
}

export interface KeysModelInput {
	me: string;
	devices: readonly DeviceRow[];
	/** `GET /devices` answered: a vault without a row is then really unlisted. */
	devicesLoaded: boolean;
	keys: readonly KeySessionSnapshot[];
	local: LocalSummary;
	live?(deviceId: string): LiveState | undefined;
	/** undefined: not read yet (older hub interim, or the list is still loading). */
	accountBackup(deviceId: string): AccountBackupFact | undefined;
	accessRequests: readonly AccessRequestRecord[];
}

export interface KeysModel {
	/** Devices of the list this computer could hold keys for, riskiest first. */
	rows: KeyRow[];
	localOnly: LocalOnlyRow[];
	totalDevices: number;
	/** Rows with keys here, plus the keys of revoked devices. */
	keysHere: number;
	groups: Record<KeyGroup, KeyRow[]>;
	unchecked: KeyRow[];
}

const RANK: Record<KeyCategory, number> = {
	never: 0,
	lost: 1,
	hub_newer: 2,
	pending: 3,
	oldpw: 4,
	out_of_date: 5,
	unchecked: 6,
	restorable: 7,
	synced: 8,
	nokeys: 9,
};

const GROUP: Partial<Record<KeyCategory, KeyGroup>> = {
	synced: "synced",
	pending: "behind",
	oldpw: "behind",
	out_of_date: "behind",
	hub_newer: "behind",
	never: "never",
	restorable: "restorable",
	lost: "nokeys",
	nokeys: "nokeys",
};

export function keyGroupOf(category: KeyCategory): KeyGroup | undefined {
	return GROUP[category];
}

const MANAGEABLE: readonly Relationship[] = ["owner", "shared"];
const NO_BACKUP: LocalBackup = { localRevision: 0, pending: false };

function withoutKeys(
	relationship: Relationship,
	hubRevision: number | undefined,
) {
	const onAccount = hubRevision !== undefined && hubRevision > 0;
	if (!MANAGEABLE.includes(relationship))
		return onAccount ? "restorable" : "nokeys";
	if (hubRevision === undefined) return "unchecked";
	return onAccount ? "restorable" : "lost";
}

function withKeys(backup: LocalBackup, hubRevision: number | undefined) {
	if (backup.pending) return "pending";
	if (hubRevision === undefined) return "unchecked";
	if (hubRevision === 0) return "never";
	if (backup.passwordChangedSinceBackup) return "oldpw";
	if (hubRevision > backup.localRevision) return "hub_newer";
	return backup.sourceDigestChanged ? "out_of_date" : "synced";
}

/** One device's state; a consent-only device never holds keys. */
export function keyCategory(row: {
	relationship: Relationship;
	hasVault: boolean;
	backup?: LocalBackup;
	hubRevision?: number;
}): KeyCategory {
	if (row.relationship === "cloud_approval") return "nokeys";
	return row.hasVault
		? withKeys(row.backup ?? NO_BACKUP, row.hubRevision)
		: withoutKeys(row.relationship, row.hubRevision);
}

function compareRows(a: KeyRow, b: KeyRow): number {
	return (
		RANK[a.category] - RANK[b.category] ||
		a.name.localeCompare(b.name) ||
		a.deviceId.localeCompare(b.deviceId)
	);
}

function keyRow(input: KeysModelInput, row: DeviceRow): KeyRow {
	const deviceId = row.device_id;
	const relationship = relationshipOf(row, input.me);
	const vault = input.local.vaults.find((entry) => entry.deviceId === deviceId);
	const backup = input.local.backups[deviceId];
	const account = input.accountBackup(deviceId);
	const live = input.live?.(deviceId);
	return {
		deviceId,
		name: row.display_name || row.name,
		row,
		relationship,
		session: input.keys.find((entry) => entry.deviceId === deviceId),
		...(live ? { live } : {}),
		...(vault ? { vault } : {}),
		...(backup ? { backup } : {}),
		...(account ? { hubRevision: account.revision } : {}),
		...(account?.updatedAt === undefined
			? {}
			: { hubSavedAt: account.updatedAt }),
		category: keyCategory({
			relationship,
			hasVault: vault !== undefined,
			backup,
			hubRevision: account?.revision,
		}),
	};
}

function localOnlyRows(input: KeysModelInput): LocalOnlyRow[] {
	const byId = new Map(input.devices.map((row) => [row.device_id, row]));
	const rows: LocalOnlyRow[] = [];
	for (const vault of input.local.vaults) {
		const row = byId.get(vault.deviceId);
		if (row?.status === "revoked") {
			rows.push({
				deviceId: vault.deviceId,
				name: row.display_name || row.name,
				reason: "revoked",
				role: vault.role,
				registeredAt: row.registered_at,
				...(row.revoked_at ? { since: row.revoked_at } : {}),
				...(row.last_seen_at ? { lastSeenAt: row.last_seen_at } : {}),
			});
			continue;
		}
		if (row || !input.devicesLoaded) continue;
		const request = input.accessRequests.find(
			(entry) => entry.deviceId === vault.deviceId && !entry.approved,
		);
		rows.push({
			deviceId: vault.deviceId,
			name: request?.deviceName ?? vault.deviceId.slice(0, 8),
			reason: request ? "request" : "unlisted",
			role: vault.role,
			...(request ? { since: request.createdAt } : {}),
			...(request?.ownerId ? { ownerId: request.ownerId } : {}),
		});
	}
	const order: Record<LocalOnlyReason, number> = {
		revoked: 0,
		request: 1,
		unlisted: 2,
	};
	return rows.sort(
		(a, b) => order[a.reason] - order[b.reason] || a.name.localeCompare(b.name),
	);
}

/** The per-device table, the local-only keys and the summary windows of N9 (SPEC §5.9). */
export function buildKeysModel(input: KeysModelInput): KeysModel {
	const held = new Set(input.local.vaults.map((vault) => vault.deviceId));
	/* A revoked device leaves the table, except one the viewer still approves cloud access for. */
	const listed = (row: DeviceRow) =>
		row.status !== "revoked" ||
		(relationshipOf(row, input.me) === "cloud_approval" &&
			!held.has(row.device_id));
	const rows = input.devices
		.filter(listed)
		.map((row) => keyRow(input, row))
		.sort(compareRows);
	const localOnly = localOnlyRows(input);
	const groups: Record<KeyGroup, KeyRow[]> = {
		synced: [],
		behind: [],
		never: [],
		restorable: [],
		nokeys: [],
	};
	for (const row of rows) {
		const group = GROUP[row.category];
		if (group) groups[group].push(row);
	}
	return {
		rows,
		localOnly,
		totalDevices: input.devices.length,
		keysHere:
			rows.filter((row) => row.vault).length +
			localOnly.filter((row) => row.reason === "revoked").length,
		groups,
		unchecked: rows.filter((row) => row.category === "unchecked"),
	};
}

/** Devices the restore flow can offer: manageable, active and without keys here. */
export function restoreCandidates(model: KeysModel): KeyRow[] {
	return model.rows.filter(
		(row) => !row.vault && row.relationship !== "cloud_approval",
	);
}

/** Rows whose keys are on this computer (the password, download and delete flows). */
export function rowsWithKeys(model: KeysModel): KeyRow[] {
	return model.rows.filter((row) => row.vault);
}

/** This computer's version of the backup: a staged upload is the one after the last acknowledged. */
export function localRevision(row: Pick<KeyRow, "backup">): number {
	const acknowledged = row.backup?.localRevision ?? 0;
	return row.backup?.pending ? acknowledged + 1 : acknowledged;
}
