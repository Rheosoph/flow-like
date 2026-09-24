export const DEVICE_SIGNALING_PROTOCOL = "flowlike.device-management.v1";
export const DEVICE_SIGNALING_JOSE_TYPE = "flow-like-device-signaling+jwt";
export const DEVICE_SIGNALING_AUDIENCE = "flow-like-device-signaling";
export const DEVICE_SIGNALING_SCOPE = "device:signal";
export const DEVICE_TOPIC_PREFIX = "device-mgmt:v1:";
export const MAX_DEVICE_PAYLOAD_BYTES = 32 * 1024;
export const MAX_DEVICE_FRAME_BYTES = 48 * 1024;

export type DeviceRole = "device" | "controller";
export type DeviceAdmission = {
	deviceId: string;
	deviceAuthEpoch: number;
	participantId: string;
	role: DeviceRole;
	subject: string;
	expiresAtMs: number;
};
export type DeviceFrame = {
	type: "frame";
	to: string;
	channel: "signal" | "noise";
	payload: string;
};
export type RelayedDeviceFrame = DeviceFrame & {
	from: string;
	from_role: DeviceRole;
};
export type DeviceFanout = {
	type: "device-frame";
	device_id: string;
	device_auth_epoch: number;
	expires_at_ms: number;
	_origin: string;
	frame: RelayedDeviceFrame;
};

export function deviceIdentifier(value: unknown): value is string {
	return (
		typeof value === "string" &&
		/^[A-Za-z0-9_.-]{1,128}$/.test(value) &&
		value !== "." &&
		value !== ".."
	);
}

function exactKeys(
	value: unknown,
	keys: string[],
): value is Record<string, unknown> {
	return (
		typeof value === "object" &&
		value !== null &&
		!Array.isArray(value) &&
		Object.keys(value).length === keys.length &&
		keys.every((key) => Object.hasOwn(value, key))
	);
}

export function deviceInbox(
	admission: Pick<
		DeviceAdmission,
		"deviceId" | "deviceAuthEpoch" | "role" | "participantId"
	>,
): string {
	if (
		!deviceIdentifier(admission.deviceId) ||
		!deviceIdentifier(admission.participantId) ||
		!Number.isSafeInteger(admission.deviceAuthEpoch) ||
		admission.deviceAuthEpoch < 1 ||
		!["device", "controller"].includes(admission.role) ||
		(admission.role === "device" &&
			admission.participantId !== admission.deviceId)
	)
		throw new Error("Invalid device signaling inbox");
	return `${DEVICE_TOPIC_PREFIX}${admission.deviceId}:epoch:${admission.deviceAuthEpoch}:${admission.role}:${admission.participantId}`;
}

function validPayload(payload: unknown): payload is string {
	if (
		typeof payload !== "string" ||
		payload.length === 0 ||
		payload.length > Math.ceil((MAX_DEVICE_PAYLOAD_BYTES * 4) / 3) ||
		!/^[A-Za-z0-9_-]+$/.test(payload)
	)
		return false;
	const bytes = Buffer.from(payload, "base64url");
	return (
		bytes.length <= MAX_DEVICE_PAYLOAD_BYTES &&
		bytes.toString("base64url") === payload
	);
}

export function parseDeviceFrame(value: unknown): DeviceFrame {
	if (
		!exactKeys(value, ["type", "to", "channel", "payload"]) ||
		value.type !== "frame" ||
		!deviceIdentifier(value.to) ||
		!["signal", "noise"].includes(String(value.channel)) ||
		!validPayload(value.payload)
	)
		throw new Error("Invalid device signaling frame");
	return value as DeviceFrame;
}

/** Routing identity comes from admission; the payload remains opaque to the hub. */
export function relayDeviceFrame(
	admission: DeviceAdmission,
	value: unknown,
): { topic: string; frame: RelayedDeviceFrame } {
	deviceInbox(admission);
	const frame = parseDeviceFrame(value);
	if (admission.role === "controller" && frame.to !== admission.deviceId)
		throw new Error("Controllers can address only their admitted device");
	const targetRole = admission.role === "device" ? "controller" : "device";
	return {
		topic: deviceInbox({
			...admission,
			role: targetRole,
			participantId: frame.to,
		}),
		frame: {
			...frame,
			from: admission.participantId,
			from_role: admission.role,
		},
	};
}

export function parseDeviceFanout(
	raw: string,
	now = Date.now(),
): { origin: string; topic: string; frame: RelayedDeviceFrame } {
	if (Buffer.byteLength(raw) > MAX_DEVICE_FRAME_BYTES)
		throw new Error("Device fanout frame exceeds its bound");
	const value: unknown = JSON.parse(raw);
	if (
		!exactKeys(value, [
			"type",
			"device_id",
			"device_auth_epoch",
			"expires_at_ms",
			"_origin",
			"frame",
		]) ||
		value.type !== "device-frame" ||
		!deviceIdentifier(value._origin) ||
		typeof value.expires_at_ms !== "number" ||
		!Number.isSafeInteger(value.expires_at_ms) ||
		value.expires_at_ms <= now ||
		value.expires_at_ms > now + 305_000 ||
		!exactKeys(value.frame, [
			"type",
			"to",
			"channel",
			"payload",
			"from",
			"from_role",
		]) ||
		!deviceIdentifier(value.frame.from) ||
		!["device", "controller"].includes(String(value.frame.from_role))
	)
		throw new Error("Invalid device fanout envelope");
	const frame = value.frame;
	const routed = relayDeviceFrame(
		{
			deviceId: value.device_id as string,
			deviceAuthEpoch: value.device_auth_epoch as number,
			role: frame.from_role as DeviceRole,
			participantId: frame.from as string,
			subject: "fanout",
			expiresAtMs: value.expires_at_ms,
		},
		{
			type: frame.type,
			to: frame.to,
			channel: frame.channel,
			payload: frame.payload,
		},
	);
	return { origin: value._origin, ...routed };
}
