import type { TFunction } from "i18next";
import type { DeviceErrorCode } from "../../../../lib/device-management/workspace/errors";

type DevicesT = TFunction<"devices">;
type Params = Record<string, string | number>;
type Copy = (t: DevicesT, params: Params) => string;

/** IA §6.4.3 / SPEC §6: one plain sentence per error code. Technical detail goes behind "Details". */
const ERROR_COPY = {
	not_configured: (t) =>
		t(
			"devices:error.notConfigured",
			"Device connections aren't set up on this hub. The hub operator needs to configure the connection service.",
		),
	needs_wss: (t) =>
		t(
			"devices:error.needsWss",
			"The hub's connection service needs a secure (WSS) address. Ask the hub operator to fix its configuration.",
		),
	access_expired: (t) =>
		t(
			"devices:error.accessExpired",
			"Your access to this device has ended. Ask the owner to renew it.",
		),
	epoch_mismatch: (t) =>
		t(
			"devices:error.epochMismatch",
			"This device was set up again or revoked after its keys were saved here, so those keys no longer work.",
		),
	invalid_admission: (t) =>
		t(
			"devices:error.invalidAdmission",
			"The hub gave an invalid connection pass. Try again; if it repeats, contact the hub operator.",
		),
	http_error: (t, params) =>
		params.status === undefined
			? t(
					"devices:error.httpError",
					"The hub couldn't issue a connection pass. Check your connection and try again.",
				)
			: t(
					"devices:error.httpErrorStatus",
					"The hub couldn't issue a connection pass (error {{status}}). Try again in a moment.",
					{ status: params.status },
				),
	relay_unreachable: (t) =>
		t(
			"devices:error.relayUnreachable",
			"The hub's connection service couldn't be reached. Check your network, then try again.",
		),
	relay_no_turn_servers: (t) =>
		t(
			"devices:error.relayNoTurnServers",
			"Connected through the hub: the hub has no relay (TURN) servers for a direct connection.",
		),
	relay_ice_timeout: (t) =>
		t(
			"devices:error.relayIceTimeout",
			"Connected through the hub: a direct connection timed out.",
		),
	relay_webrtc_failed: (t) =>
		t(
			"devices:error.relayWebrtcFailed",
			"Connected through the hub: a direct connection failed.",
		),
	relay_webrtc_unavailable: (t) =>
		t(
			"devices:error.relayWebrtcUnavailable",
			"Connected through the hub: this browser can't make direct connections.",
		),
	handshake_failed: (t) =>
		t(
			"devices:error.handshakeFailed",
			"The secure connection couldn't be set up. The device may be restarting.",
		),
	identity_confirmation_failed: (t) =>
		t(
			"devices:error.identityConfirmationFailed",
			"The device couldn't prove its identity. Check the device before you connect again.",
		),
	session_closed: (t) =>
		t("devices:error.sessionClosed", "The connection to the device closed."),
	timeout: (t) =>
		t("devices:error.timeout", "The device didn't answer in time."),
	expired: (t) =>
		t(
			"devices:error.expired",
			"The secure session ended before the request could be sent.",
		),
	cancelled: (t) => t("devices:error.cancelled", "Connecting was cancelled."),
	connection_failed: (t) =>
		t("devices:error.connectionFailed", "The device couldn't be reached."),
	wrong_password: (t) =>
		t(
			"devices:error.wrongPassword",
			"That password doesn't open the keys for this device on this computer.",
		),
	no_vault: (t) =>
		t(
			"devices:error.noVault",
			"There are no keys for this device on this computer for this account, hub and profile.",
		),
	held_elsewhere: (t) =>
		t(
			"devices:error.heldElsewhere",
			"This device is unlocked in another tab or window.",
		),
	lock_unsupported: (t) =>
		t(
			"devices:error.lockUnsupported",
			"This browser can't make sure only one tab uses the keys at a time.",
		),
	crypto_unavailable: (t) =>
		t(
			"devices:error.cryptoUnavailable",
			"Device encryption couldn't be loaded. Refresh the app and try again.",
		),
	identity_mismatch: (t) =>
		t(
			"devices:error.identityMismatch",
			"The hub reports different keys for this device than the ones you trusted.",
		),
	authority_mismatch: (t) =>
		t(
			"devices:error.authorityMismatch",
			"The saved keys belong to another hub or account.",
		),
	storage_error: (t) =>
		t(
			"devices:error.storageError",
			"The keys on this computer couldn't be read or saved.",
		),
	keys_locked: (t) =>
		t("devices:error.keysLocked", "Unlock this device to talk to it."),
	device_unreachable: (t) =>
		t(
			"devices:error.deviceUnreachable",
			"The device is offline. Live data returns when it checks in again.",
		),
	not_sent: (t) =>
		t(
			"devices:error.notSent",
			"The request wasn't sent. Nothing changed on the device.",
		),
	no_reply: (t) =>
		t(
			"devices:error.noReply",
			"The device didn't confirm the result. Check the result before you try again.",
		),
	busy: (t) =>
		t("devices:error.busy", "Another request to this device is still running."),
	slots_in_use: (t) =>
		t(
			"devices:error.slotsInUse",
			"All connection slots for your access are in use, so the oldest one was closed.",
		),
	fleet_network: (t) =>
		t(
			"devices:error.fleetNetwork",
			"Couldn't refresh the encrypted status. Showing the last one read.",
		),
	fleet_integrity: (t) =>
		t(
			"devices:error.fleetIntegrity",
			"The encrypted status failed its integrity check, so it isn't shown.",
		),
	fleet_access: (t) =>
		t(
			"devices:error.fleetAccess",
			"Your access doesn't include the encrypted status of this device.",
		),
	rejected_unauthorized: (t) =>
		t(
			"devices:error.rejectedUnauthorized",
			"Your access doesn't allow this on the device.",
		),
	rejected_revision_conflict: (t) =>
		t(
			"devices:error.rejectedRevisionConflict",
			"The device changed since it was last read. Refresh, then try again.",
		),
	rejected_invalid: (t) =>
		t(
			"devices:error.rejectedInvalid",
			"The device refused the request as invalid.",
		),
	rejected_host_policy: (t) =>
		t(
			"devices:error.rejectedHostPolicy",
			"The device's host policy doesn't allow this.",
		),
	rejected_unsupported: (t) =>
		t(
			"devices:error.rejectedUnsupported",
			"The device agent is too old for this. Update the agent to use it.",
		),
	rejected_limit: (t) =>
		t(
			"devices:error.rejectedLimit",
			"The device reached a limit for this kind of request.",
		),
	rejected_busy: (t) =>
		t(
			"devices:error.rejectedBusy",
			"The device is busy. Try again in a moment.",
		),
	rejected_failed: (t) =>
		t(
			"devices:error.rejectedFailed",
			"The device couldn't complete the request.",
		),
	rejected: (t) =>
		t("devices:error.rejected", "The device rejected the request."),
} satisfies Record<DeviceErrorCode, Copy>;

export function errorCopy(
	t: DevicesT,
	code: DeviceErrorCode,
	params: Params = {},
): string {
	return ERROR_COPY[code](t, params);
}
