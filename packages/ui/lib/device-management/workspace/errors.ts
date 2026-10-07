import {
	ConnectError,
	type ConnectErrorCode,
	type ManagementFailureDiagnostic,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "../transport";
import { DeviceTunnelError } from "../tunnel";
import type { ManagementRejection } from "../types";
import type {
	FleetDeviceState,
	KeyError,
	KeyHubError,
	LiveError,
	RelayFallbackReason,
	UnlockStepId,
} from "./types";

/** IA §6.4.3: every connection error is filed under one of these seven steps. */
export const CONNECTION_STEPS = [
	"unlocking_keys",
	"checking_identity",
	"getting_pass",
	"reaching_device",
	"trying_direct",
	"securing",
	"reading_services",
] as const satisfies readonly UnlockStepId[];
export type ConnectionStep = (typeof CONNECTION_STEPS)[number];

export const CONNECT_ERROR_CODES = [
	"not_configured",
	"needs_wss",
	"access_expired",
	"epoch_mismatch",
	"invalid_admission",
	"http_error",
	"relay_unreachable",
	"relay_no_turn_servers",
	"relay_ice_timeout",
	"relay_webrtc_failed",
	"relay_webrtc_unavailable",
	"handshake_failed",
	"identity_confirmation_failed",
	"session_closed",
	"timeout",
	"expired",
	"cancelled",
	"connection_failed",
] as const;

export const KEY_ERROR_CODES = [
	"wrong_password",
	"no_vault",
	"held_elsewhere",
	"lock_unsupported",
	"crypto_unavailable",
	"identity_mismatch",
	"authority_mismatch",
	"storage_error",
] as const;

export const LIVE_ERROR_CODES = [
	"keys_locked",
	"device_unreachable",
	"not_sent",
	"no_reply",
	"busy",
	"slots_in_use",
] as const;

export const FLEET_ERROR_CODES = [
	"fleet_network",
	"fleet_integrity",
	"fleet_access",
] as const;

export const REJECTION_ERROR_CODES = [
	"rejected_unauthorized",
	"rejected_revision_conflict",
	"rejected_invalid",
	"rejected_host_policy",
	"rejected_unsupported",
	"rejected_limit",
	"rejected_busy",
	"rejected_failed",
	"rejected",
] as const;

export const DEVICE_ERROR_CODES = [
	...CONNECT_ERROR_CODES,
	...KEY_ERROR_CODES,
	...LIVE_ERROR_CODES,
	...FLEET_ERROR_CODES,
	...REJECTION_ERROR_CODES,
] as const;
export type DeviceErrorCode = (typeof DEVICE_ERROR_CODES)[number];

export interface DeviceFailure {
	step: ConnectionStep;
	code: DeviceErrorCode;
	/** The original sentence, for "Details" and Copy diagnostics only. */
	message: string;
	status?: number;
	url?: string;
	rejection?: ManagementRejection;
}

const RELAY_FALLBACK: Record<RelayFallbackReason, DeviceErrorCode> = {
	no_turn_servers: "relay_no_turn_servers",
	ice_timeout: "relay_ice_timeout",
	webrtc_failed: "relay_webrtc_failed",
	webrtc_unavailable: "relay_webrtc_unavailable",
};

export function fallbackCode(reason: RelayFallbackReason): DeviceErrorCode {
	return RELAY_FALLBACK[reason];
}

const CONNECT_CODE: Record<ConnectErrorCode, DeviceErrorCode> = {
	not_configured: "not_configured",
	needs_wss: "needs_wss",
	access_expired: "access_expired",
	epoch_mismatch: "epoch_mismatch",
	invalid_admission: "invalid_admission",
	http: "http_error",
	relay_unreachable: "relay_unreachable",
	handshake_failed: "handshake_failed",
	identity_confirmation_failed: "identity_confirmation_failed",
	cancelled: "cancelled",
};

const TUNNEL_CODE = new Map<string, DeviceErrorCode>([
	["connection_closed", "session_closed"],
	["heartbeat_timeout", "timeout"],
	["renewal_timeout", "timeout"],
	["open_timeout", "timeout"],
	["expired", "expired"],
	["cancelled", "cancelled"],
]);

type Filed = readonly [ConnectionStep, DeviceErrorCode];

/** Every sentence the transport throws (DM/transport.ts, DM/crypto.ts), filed under its step. */
const TRANSPORT_MESSAGES: Record<string, Filed> = {
	"Invalid device signaling admission.": ["getting_pass", "invalid_admission"],
	"Management connection cancelled.": ["getting_pass", "cancelled"],
	"Invalid device signaling endpoint.": [
		"reaching_device",
		"relay_unreachable",
	],
	"Invalid signaling frame.": ["reaching_device", "relay_unreachable"],
	"Unexpected signaling frame.": ["reaching_device", "relay_unreachable"],
	"Missing signaling payload.": ["reaching_device", "relay_unreachable"],
	"Device signaling protocol differs.": [
		"reaching_device",
		"relay_unreachable",
	],
	"Device signaling admission differs.": [
		"reaching_device",
		"relay_unreachable",
	],
	"Device signaling could not be reached.": [
		"reaching_device",
		"relay_unreachable",
	],
	"WebRTC is unavailable.": ["trying_direct", "relay_webrtc_unavailable"],
	"WebRTC connection failed.": ["trying_direct", "relay_webrtc_failed"],
	"Invalid device ICE configuration.": ["trying_direct", "relay_webrtc_failed"],
	"WebRTC answer does not match this session.": [
		"trying_direct",
		"relay_webrtc_failed",
	],
	"Unexpected Noise handshake.": ["securing", "handshake_failed"],
	"Device did not confirm encrypted management.": [
		"securing",
		"handshake_failed",
	],
	"Invalid management frame.": ["securing", "handshake_failed"],
	"Management frame exceeds its bound.": ["securing", "handshake_failed"],
	"Invalid management envelope.": ["securing", "handshake_failed"],
	"Invalid management encoding.": ["securing", "handshake_failed"],
	"Encrypted device identity confirmation failed.": [
		"securing",
		"identity_confirmation_failed",
	],
	"Management connection timed out.": ["reading_services", "timeout"],
	"Management connection closed.": ["reading_services", "session_closed"],
	"Management input exceeded its bound.": [
		"reading_services",
		"session_closed",
	],
	"Management frames require an ordered reader.": [
		"reading_services",
		"session_closed",
	],
	"Management connection is unavailable or busy.": [
		"reading_services",
		"session_closed",
	],
	"WebRTC channel is unavailable or busy.": [
		"reading_services",
		"session_closed",
	],
	"Management response does not match this session.": [
		"reading_services",
		"session_closed",
	],
	"Management response does not match the requested operation.": [
		"reading_services",
		"session_closed",
	],
	"Management session expired. Reconnect before continuing.": [
		"reading_services",
		"expired",
	],
	"Wait for the current device operation to finish.": [
		"reading_services",
		"busy",
	],
	"Device signaling renewal did not extend admission.": [
		"getting_pass",
		"invalid_admission",
	],
	"Device signaling renewal differs.": ["reaching_device", "relay_unreachable"],
	"Device tunnel transport stalled.": ["reading_services", "timeout"],
	"Device tunnel transport closed.": ["reading_services", "session_closed"],
	"Device data transport is unavailable.": [
		"reading_services",
		"session_closed",
	],
	"This operation requires the management connection.": [
		"reading_services",
		"session_closed",
	],
	"The device data request could not be sent.": [
		"reading_services",
		"not_sent",
	],
	"Update the device agent to connect to deployed services.": [
		"reading_services",
		"rejected_unsupported",
	],
	"Update the device agent to use encrypted streaming transfers.": [
		"reading_services",
		"rejected_unsupported",
	],
	"Update the device agent to send requests to its models.": [
		"reading_services",
		"rejected_unsupported",
	],
};

const INSPECTION_REJECTED =
	"This controller cannot read device placement status.";

export function rejectionCode(
	rejection?: Pick<ManagementRejection, "code">,
): DeviceErrorCode {
	const code = `rejected_${rejection?.code ?? ""}`;
	return (REJECTION_ERROR_CODES as readonly string[]).includes(code)
		? (code as DeviceErrorCode)
		: "rejected";
}

/** Thrown by the live manager's `call()` when a request cannot be served. */
export class LiveCallError extends Error {
	constructor(
		readonly code: DeviceErrorCode,
		message: string,
		readonly liveError?: LiveError,
		readonly diagnostic?: ManagementFailureDiagnostic,
	) {
		super(message);
		this.name = "LiveCallError";
	}
}

/** Files any error from connecting or calling a device under one step and one code. */
export function classifyDeviceError(
	error: unknown,
	during: ConnectionStep = "reading_services",
): DeviceFailure {
	const message = error instanceof Error ? error.message : String(error);
	if (error instanceof ConnectError)
		return {
			step: error.step,
			code: CONNECT_CODE[error.code],
			message,
			status: error.detail.status,
			url: error.detail.url,
		};
	if (error instanceof LiveCallError)
		return {
			...(error.liveError
				? liveFailure(error.liveError)
				: { step: during, code: error.code }),
			message,
		};
	if (error instanceof ManagementRequestNotSentError)
		return { step: "reading_services", code: "not_sent", message };
	if (error instanceof ManagementUnconfirmedError)
		return { step: "reading_services", code: "no_reply", message };
	const tunnelCode =
		error instanceof DeviceTunnelError
			? TUNNEL_CODE.get(error.code)
			: undefined;
	if (tunnelCode)
		return {
			step: "reading_services",
			code: tunnelCode,
			message,
		};
	const filed = TRANSPORT_MESSAGES[message];
	if (filed) return { step: filed[0], code: filed[1], message };
	if (message.startsWith(INSPECTION_REJECTED))
		return { step: "reading_services", code: "rejected", message };
	return { step: during, code: "connection_failed", message };
}

/** Whether the sentence is one the transport is known to throw. */
export function isKnownTransportMessage(message: string): boolean {
	return message in TRANSPORT_MESSAGES;
}

function liveFailure(cause: LiveError): Omit<DeviceFailure, "message"> {
	switch (cause.step) {
		case "getting_pass":
			return {
				step: "getting_pass",
				code: cause.code === "http" ? "http_error" : cause.code,
				status: cause.status,
			};
		case "reaching_device":
			return { step: "reaching_device", code: cause.code, url: cause.url };
		case "securing":
			return { step: "securing", code: cause.code };
		case "reading_services":
			return {
				step: "reading_services",
				code: rejectionCode(cause.rejection),
				rejection: cause.rejection,
			};
		case "session":
			return {
				step: "reading_services",
				code: cause.code === "closed" ? "session_closed" : cause.code,
			};
	}
}

/** The code shown for a live session cause (LiveState `reconnecting` / `unreachable` / `failed`). */
export function liveErrorCode(cause: LiveError): DeviceErrorCode {
	return liveFailure(cause).code;
}

function connectLiveError(error: ConnectError): LiveError | undefined {
	if (error.code === "cancelled") return undefined;
	if (error.step === "getting_pass")
		return {
			step: "getting_pass",
			code: error.code as Extract<LiveError, { step: "getting_pass" }>["code"],
			status: error.detail.status,
		};
	if (error.step === "reaching_device")
		return {
			step: "reaching_device",
			code: "relay_unreachable",
			url: error.detail.url,
		};
	return {
		step: "securing",
		code:
			error.code === "identity_confirmation_failed"
				? "identity_confirmation_failed"
				: "handshake_failed",
	};
}

const SESSION_CODE: Partial<
	Record<DeviceErrorCode, Extract<LiveError, { step: "session" }>["code"]>
> = { timeout: "timeout", expired: "expired" };

/** Contract shape for a connect or session failure, or `undefined` for a cancellation. */
export function toLiveError(error: unknown): LiveError | undefined {
	if (error instanceof LiveCallError && error.liveError) return error.liveError;
	if (error instanceof ConnectError) return connectLiveError(error);
	const code = classifyDeviceError(error).code;
	return { step: "session", code: SESSION_CODE[code] ?? "closed" };
}

/** Access ended, re-enrolled or revoked, or the device's identity failed: retrying cannot help. */
export function isFatalLiveError(cause: LiveError): boolean {
	return (
		(cause.step === "getting_pass" &&
			(cause.code === "access_expired" || cause.code === "epoch_mismatch")) ||
		(cause.step === "securing" && cause.code === "identity_confirmation_failed")
	);
}

type SessionKeyError = Pick<KeyError | KeyHubError, "code">;

const KEY_CODE: Record<SessionKeyError["code"], DeviceErrorCode> = {
	wrong_password: "wrong_password",
	no_vault: "no_vault",
	held_elsewhere: "held_elsewhere",
	lock_unsupported: "lock_unsupported",
	crypto_unavailable: "crypto_unavailable",
	identity_mismatch: "identity_mismatch",
	authority_mismatch: "authority_mismatch",
	storage: "storage_error",
	hub: "http_error",
};

const IDENTITY_STEP_CODES: ReadonlySet<SessionKeyError["code"]> = new Set([
	"identity_mismatch",
	"authority_mismatch",
	"hub",
]);

export function keyErrorCode(error: SessionKeyError): DeviceErrorCode {
	return KEY_CODE[error.code];
}

export function keyErrorStep(error: SessionKeyError): ConnectionStep {
	return IDENTITY_STEP_CODES.has(error.code)
		? "checking_identity"
		: "unlocking_keys";
}

const FLEET_CODE: Record<
	NonNullable<FleetDeviceState["error"]>["kind"],
	DeviceErrorCode
> = {
	network: "fleet_network",
	integrity: "fleet_integrity",
	access: "fleet_access",
};

export function fleetErrorCode(
	error: Pick<NonNullable<FleetDeviceState["error"]>, "kind">,
): DeviceErrorCode {
	return FLEET_CODE[error.kind];
}
