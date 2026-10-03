import type { AttentionInputExt, AttentionRuleExt } from "../attention";
import {
	DAY_S,
	type DeviceFacts,
	attentionCandidate,
	deviceLabel,
	localSource,
	perDevice,
} from "../device-view";
import type { DevicesRoute } from "../types";
import { pendingAccessRequests } from "./access";

const ABANDONED_REQUEST_S = 30 * DAY_S;
/** IA §6.5: 240 of 256 account backup slots. */
const BACKUP_SLOTS_RATIO = 240 / 256;

const keysRoute = (deviceId?: string): DevicesRoute =>
	deviceId ? { screen: "keys", focusDeviceId: deviceId } : { screen: "keys" };

const keysSubject = (deviceId: string) => ({ kind: "keys", deviceId }) as const;

/** Key holder of a device the hub still lists as active (owner or approved recipient). */
function holdsKeys(device: DeviceFacts) {
	return (
		device.active &&
		!!device.vault &&
		(device.relationship === "owner" || device.relationship === "shared")
	);
}

/** Account backup revision on the hub; undefined when the hub list doesn't say. */
function hubRevision(input: AttentionInputExt, deviceId: string) {
	const entry = input.accountBackups[deviceId];
	if (entry) return entry.revision;
	return input.accountBackupSlots ? 0 : undefined;
}

const keysMissing = perDevice("keys_missing_here", (input, device) => {
	if (
		!device.active ||
		device.vault ||
		device.keys.state !== "none" ||
		(device.relationship !== "owner" && device.relationship !== "shared")
	)
		return undefined;
	const owner = device.relationship === "owner";
	return attentionCandidate({
		key: "keys_missing_here",
		severity: owner ? "warning" : "notice",
		subject: keysSubject(device.id),
		params: { device: device.name },
		action: {
			code: "restore_keys",
			target: { kind: "restore_keys", deviceId: device.id },
		},
		source: localSource(input),
	});
});

const notBackedUp = perDevice(
	"keys_not_backed_up_to_account",
	(input, device) => {
		if (
			!holdsKeys(device) ||
			hubRevision(input, device.id) !== 0 ||
			input.local.backups[device.id]?.pending
		)
			return undefined;
		return attentionCandidate({
			key: "keys_not_backed_up_to_account",
			severity: "warning",
			subject: keysSubject(device.id),
			params: { device: device.name },
			action: { code: "back_up_to_account", target: keysRoute(device.id) },
			source: localSource(input),
			since: device.row.registered_at,
		});
	},
);

const uploadPending = perDevice(
	"account_backup_upload_pending",
	(input, device) => {
		const backup = input.local.backups[device.id];
		if (!holdsKeys(device) || !backup?.pending) return undefined;
		return attentionCandidate({
			key: "account_backup_upload_pending",
			severity: "notice",
			subject: keysSubject(device.id),
			params: { device: device.name, revision: backup.localRevision },
			action: { code: "retry_upload", target: keysRoute(device.id) },
			source: localSource(input),
		});
	},
);

const oldPassword = perDevice(
	"account_backup_old_password",
	(input, device) => {
		const backup = input.local.backups[device.id];
		if (
			!holdsKeys(device) ||
			!backup?.passwordChangedSinceBackup ||
			(hubRevision(input, device.id) ?? 0) === 0
		)
			return undefined;
		return attentionCandidate({
			key: "account_backup_old_password",
			severity: "notice",
			subject: keysSubject(device.id),
			params: { device: device.name },
			action: { code: "update_account_backup", target: keysRoute(device.id) },
			source: localSource(input),
		});
	},
);

const hubNewer = perDevice("account_backup_hub_newer", (input, device) => {
	const local = input.local.backups[device.id];
	const hub = input.accountBackups[device.id];
	if (
		!holdsKeys(device) ||
		!local ||
		!hub ||
		hub.revision <= local.localRevision
	)
		return undefined;
	return attentionCandidate({
		key: "account_backup_hub_newer",
		severity: "warning",
		subject: keysSubject(device.id),
		params: {
			device: device.name,
			hubRevision: hub.revision,
			localRevision: local.localRevision,
		},
		action: { code: "review_backups", target: keysRoute(device.id) },
		source: localSource(input),
		since: hub.updatedAt,
	});
});

const storageNotPersistent: AttentionRuleExt = {
	key: "storage_not_persistent",
	evaluate(input) {
		const { local } = input;
		if (
			local.platform !== "web" ||
			local.vaults.length === 0 ||
			(local.persistence !== "denied" && local.persistence !== "unavailable")
		)
			return [];
		return [
			attentionCandidate({
				key: "storage_not_persistent",
				severity: "warning",
				subject: { kind: "keys" },
				params: { count: local.vaults.length },
				action: {
					code: "keep_keys_safely",
					target: { kind: "keep_keys_safely" },
				},
				source: localSource(input),
			}),
		];
	},
};

const requestUnbacked: AttentionRuleExt = {
	key: "request_keys_unbacked",
	evaluate(input) {
		return pendingAccessRequests(input).flatMap((request) =>
			input.local.vaults.some((vault) => vault.deviceId === request.deviceId) &&
			!input.accountBackups[request.deviceId]?.revision
				? [
						attentionCandidate({
							key: "request_keys_unbacked",
							severity: "info",
							subject: keysSubject(request.deviceId),
							params: { device: deviceLabel(input, request.deviceId) },
							action: {
								code: "download_request_again",
								target: { screen: "access", tab: "shared" },
							},
							source: localSource(input),
							since: request.createdAt,
						}),
					]
				: [],
		);
	},
};

const backupSlots: AttentionRuleExt = {
	key: "backup_slots_nearly_full",
	evaluate(input) {
		const slots = input.accountBackupSlots;
		if (!slots || slots.max <= 0 || slots.used < slots.max * BACKUP_SLOTS_RATIO)
			return [];
		return [
			attentionCandidate({
				key: "backup_slots_nearly_full",
				severity: "notice",
				subject: { kind: "keys" },
				params: { used: slots.used, max: slots.max },
				action: { code: "review_backups", target: keysRoute() },
				source: localSource(input),
			}),
		];
	},
};

const staleKeys: AttentionRuleExt = {
	key: "stale_local_keys",
	evaluate(input) {
		const revoked = input.devices
			.filter(
				(row) =>
					row.status === "revoked" &&
					input.local.vaults.some((vault) => vault.deviceId === row.device_id),
			)
			.map((row) => ({
				deviceId: row.device_id,
				since: row.revoked_at ?? undefined,
			}));
		const abandoned = pendingAccessRequests(input)
			.filter((request) => input.now - request.createdAt > ABANDONED_REQUEST_S)
			.map((request) => ({
				deviceId: request.deviceId,
				since: request.createdAt,
			}));
		return [...revoked, ...abandoned].map(({ deviceId, since }) =>
			attentionCandidate({
				key: "stale_local_keys",
				severity: "info",
				subject: keysSubject(deviceId),
				params: { device: deviceLabel(input, deviceId) },
				action: { code: "delete_keys", target: keysRoute(deviceId) },
				source: localSource(input),
				since,
			}),
		);
	},
};

export const KEY_RULES: readonly AttentionRuleExt[] = [
	keysMissing,
	notBackedUp,
	uploadPending,
	oldPassword,
	hubNewer,
	storageNotPersistent,
	requestUnbacked,
	backupSlots,
	staleKeys,
];
