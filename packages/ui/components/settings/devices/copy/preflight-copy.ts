import type { PreflightCode } from "../../../../lib/device-management/model/types";
import type { PreflightRow } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import {
	type CopyContext,
	type CopyFormat,
	copyContext,
	fixLabel,
} from "./gate-copy";

interface RowContext extends CopyContext {
	row: PreflightRow;
}

/** IA §6.4.3 pass and fail wording, one entry per pre-flight code. */
const PREFLIGHT_COPY = {
	checking: ({ t }) => t("devices:preflight.checking", "Checking…"),
	hub_ready: ({ t }) => t("devices:preflight.hubReady", "Hub ready"),
	hub_devices_off: ({ t }) =>
		t("devices:preflight.hubDevicesOff", "Devices are off on this hub"),
	hub_unreachable: ({ t }) =>
		t("devices:preflight.hubUnreachable", "The hub can't be reached"),
	signed_in: ({ t }) =>
		t("devices:preflight.signedIn", "Signed in with full permissions"),
	sign_in_required: ({ t }) =>
		t("devices:preflight.signInRequired", "Sign in to continue"),
	token_restricted: ({ t }) =>
		t("devices:preflight.tokenRestricted", "Use a token with full permissions"),
	access_owner: ({ t }) => t("devices:preflight.accessOwner", "Owner"),
	access_shared: ({ t, at }) =>
		at("endsAt")
			? t("devices:preflight.accessSharedUntil", "Shared · ends {{endsAt}}", {
					endsAt: at("endsAt"),
				})
			: t("devices:preflight.accessShared", "Shared with you"),
	access_cloud_approval: ({ t }) =>
		t(
			"devices:preflight.accessCloudApproval",
			"You only hold a cloud approval on this device",
		),
	access_unknown: ({ t, row }) =>
		row.status === "fail"
			? t(
					"devices:preflight.accessNotListed",
					"This device isn't in your device list on this hub",
				)
			: t("devices:preflight.accessUnknown", "Shared or cloud approvals"),
	access_ended: ({ t, at }) =>
		at("at")
			? t("devices:preflight.accessEndedAt", "Your access ended on {{at}}", {
					at: at("at"),
				})
			: t("devices:preflight.accessEnded", "Your access has ended"),
	device_revoked: ({ t, at }) =>
		at("at")
			? t("devices:preflight.deviceRevokedAt", "Device revoked on {{at}}", {
					at: at("at"),
				})
			: t("devices:preflight.deviceRevoked", "Device revoked"),
	keys_owner_here: ({ t }) =>
		t("devices:preflight.keysOwnerHere", "Owner keys on this computer"),
	keys_shared_here: ({ t }) =>
		t("devices:preflight.keysSharedHere", "Your access keys on this computer"),
	keys_missing: ({ t }) =>
		t(
			"devices:preflight.keysMissing",
			"No keys here for this account, hub and profile. They may be in another browser, app profile or hub.",
		),
	keys_unusable: ({ t }) =>
		t(
			"devices:preflight.keysUnusable",
			"The keys on this computer can't be used any more",
		),
	browser_ready: ({ t }) =>
		t("devices:preflight.browserReady", "This app can protect device keys"),
	browser_cannot_protect_keys: ({ t }) =>
		t(
			"devices:preflight.browserCannotProtectKeys",
			"This browser can't protect keys",
		),
	browser_may_delete_keys: ({ t }) =>
		t("devices:preflight.browserMayDeleteKeys", "This browser may delete keys"),
	crypto_unavailable: ({ t }) =>
		t(
			"devices:preflight.cryptoUnavailable",
			"Device encryption couldn't load in this browser",
		),
	lock_free: ({ t }) =>
		t("devices:preflight.lockFree", "Not unlocked in another window"),
	lock_held_here: ({ t }) =>
		t("devices:preflight.lockHeldHere", "Unlocked in this window"),
	lock_held_elsewhere: ({ t }) =>
		t("devices:preflight.lockHeldElsewhere", "Unlocked in another window"),
	lock_unsupported: ({ t }) =>
		t(
			"devices:preflight.lockUnsupported",
			"This browser can't protect device keys. Use the desktop app or a current browser.",
		),
	identity_trusted: ({ t, p, at }) => {
		const since = at("since");
		const fingerprint = typeof p.fingerprint === "string" ? p.fingerprint : "";
		if (since && fingerprint)
			return t(
				"devices:preflight.identityTrustedFingerprint",
				"Trusted since {{since}} · fingerprint {{fingerprint}}",
				{ since, fingerprint },
			);
		return since
			? t("devices:preflight.identityTrustedSince", "Trusted since {{since}}", {
					since,
				})
			: t("devices:preflight.identityTrusted", "Trusted identity");
	},
	identity_first_use: ({ t, p }) =>
		typeof p.fingerprint === "string"
			? t(
					"devices:preflight.identityFirstUseFingerprint",
					"First connection: its identity (fingerprint {{fingerprint}}) will be trusted from now on",
					{ fingerprint: p.fingerprint },
				)
			: t(
					"devices:preflight.identityFirstUse",
					"First connection: its identity will be trusted from now on",
				),
	identity_mismatch: ({ t, at }) =>
		at("since")
			? t(
					"devices:preflight.identityMismatchSince",
					"The hub reports different keys for this device than the ones you trusted on {{since}}. The device may have been set up again, or someone may be impersonating it.",
					{ since: at("since") },
				)
			: t(
					"devices:preflight.identityMismatch",
					"The hub reports different keys for this device than the ones you trusted. The device may have been set up again, or someone may be impersonating it.",
				),
	checkin_online: ({ t }) => t("devices:preflight.checkinOnline", "Online"),
	checkin_late: ({ t, at }) =>
		t("devices:preflight.checkinLate", "Late · last check-in {{since}}", {
			since: at("since"),
		}),
	checkin_offline: ({ t, at }) =>
		t(
			"devices:preflight.checkinOffline",
			"Offline since {{since}}. A live connection will likely fail; encrypted snapshots still work.",
			{ since: at("since") },
		),
	checkin_never: ({ t }) =>
		t(
			"devices:preflight.checkinNever",
			"Never checked in. A live connection will likely fail.",
		),
	clock_ok: ({ t }) => t("devices:preflight.clockOk", "Clocks agree"),
	clock_computer_off: ({ t, p }) =>
		t(
			"devices:preflight.clockComputerOff",
			"This computer's clock is {{minutes, number}} min off",
			{ minutes: p.minutes },
		),
	clock_device_off: ({ t, p, device }) =>
		t(
			"devices:preflight.clockDeviceOff",
			"{{device}}'s clock looks {{minutes, number}} min off",
			{ device, minutes: p.minutes },
		),
	clock_unknown: ({ t }) =>
		t(
			"devices:preflight.clockUnknown",
			"Clocks not compared yet (the hub hasn't sent its time)",
		),
	connection_ready: ({ t }) =>
		t("devices:preflight.connectionReady", "Connection service reachable"),
	connection_not_configured: ({ t }) =>
		t(
			"devices:preflight.connectionNotConfigured",
			"The hub's connection service isn't set up",
		),
	connection_needs_wss: ({ t }) =>
		t(
			"devices:preflight.connectionNeedsWss",
			"The hub's connection service needs a secure (WSS) address",
		),
	connection_access_expired: ({ t }) =>
		t("devices:preflight.connectionAccessExpired", "Your access expired"),
	connection_invalid_admission: ({ t, p }) =>
		typeof p.status === "number"
			? t(
					"devices:preflight.connectionInvalidAdmissionStatus",
					"The hub couldn't issue a connection pass (error {{status}})",
					{ status: p.status },
				)
			: t(
					"devices:preflight.connectionInvalidAdmission",
					"The hub gave an invalid connection pass",
				),
	relay_unreachable: ({ t, p }) =>
		typeof p.url === "string"
			? t(
					"devices:preflight.relayUnreachableUrl",
					"Relay unreachable ({{url}})",
					{
						url: p.url,
					},
				)
			: t("devices:preflight.relayUnreachable", "Relay unreachable"),
	connection_failed: ({ t }) =>
		t(
			"devices:preflight.connectionFailed",
			"The connection couldn't be set up",
		),
	policy_applied: ({ t }) =>
		t("devices:preflight.policyApplied", "Your access is active on the device"),
	policy_waiting: ({ t }) =>
		t(
			"devices:preflight.policyWaiting",
			"The device hasn't applied the change that gave you access yet (usually within 5 min while online)",
		),
	policy_unknown: ({ t }) =>
		t(
			"devices:preflight.policyUnknown",
			"Not yet confirmed that the device applied your access",
		),
	slots_available: ({ t }) =>
		t("devices:preflight.slotsAvailable", "Connection slots free"),
	slots_full: ({ t, p }) =>
		typeof p.limit === "number"
			? t(
					"devices:preflight.slotsFullLimit",
					"All {{limit, number}} connection slots for your access are in use; the oldest will be closed",
					{ limit: p.limit },
				)
			: t(
					"devices:preflight.slotsFull",
					"All connection slots for your access are in use; the oldest will be closed",
				),
	agent_features_ok: ({ t, p }) =>
		typeof p.version === "string"
			? t("devices:preflight.agentVersion", "Agent {{version}}", {
					version: p.version,
				})
			: t("devices:preflight.agentFeaturesOk", "Agent supports what's needed"),
	agent_features_missing: ({ t, p }) =>
		t(
			"devices:preflight.agentFeaturesMissing",
			"Some features need a newer agent ({{missing, number}} missing)",
			{ missing: p.count },
		),
	agent_features_unknown: ({ t }) =>
		t(
			"devices:preflight.agentFeaturesUnknown",
			"The agent hasn't reported its features yet",
		),
} satisfies Record<PreflightCode, (c: RowContext) => string>;

export interface PreflightCopy {
	text: string;
	/** Label of the primary fix. */
	fix?: string;
	/** Labels of the further fixes, in order (D4: Import · Request access). */
	otherFixes?: string[];
}

/** Translates one pre-flight row; codes and D-numbers stay in Copy diagnostics only (R3). */
export function preflightCopy(
	t: DevicesT,
	row: PreflightRow,
	fmt?: CopyFormat,
): PreflightCopy {
	const text = PREFLIGHT_COPY[row.copy.code]({
		...copyContext(t, row.copy.params, fmt),
		row,
	});
	return {
		text,
		...(row.fix ? { fix: fixLabel(t, row.fix) } : {}),
		...(row.otherFixes?.length
			? { otherFixes: row.otherFixes.map((fix) => fixLabel(t, fix)) }
			: {}),
	};
}
