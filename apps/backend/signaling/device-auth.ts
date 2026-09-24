import { importSPKI, jwtVerify } from "jose";
import {
	REALTIME_TOKEN_PROTOCOL_PREFIX,
	type RealtimeAuthConfig,
	RealtimeAuthError,
	originIsAllowed,
} from "./auth";
import {
	DEVICE_SIGNALING_AUDIENCE,
	DEVICE_SIGNALING_JOSE_TYPE,
	DEVICE_SIGNALING_PROTOCOL,
	DEVICE_SIGNALING_SCOPE,
	type DeviceAdmission,
	deviceIdentifier,
	deviceInbox,
} from "./device-frames";

const MAX_TOKEN_BYTES = 4096;
const TOKEN_LIFETIME_SECONDS = 300;
const CLAIMS = [
	"iss",
	"aud",
	"sub",
	"iat",
	"nbf",
	"exp",
	"jti",
	"typ",
	"scope",
	"device_id",
	"device_auth_epoch",
	"role",
	"participant_id",
];

function deviceToken(protocols: string | null): string {
	if (!protocols || protocols.length > MAX_TOKEN_BYTES + 256)
		throw new RealtimeAuthError();
	const values = protocols.split(",").map((value) => value.trim());
	if (values.length !== 2 || !values.includes(DEVICE_SIGNALING_PROTOCOL))
		throw new RealtimeAuthError();
	const token = values
		.find((value) => value.startsWith(REALTIME_TOKEN_PROTOCOL_PREFIX))
		?.slice(REALTIME_TOKEN_PROTOCOL_PREFIX.length);
	if (
		!token ||
		token.length > MAX_TOKEN_BYTES ||
		!/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(token)
	)
		throw new RealtimeAuthError();
	for (const segment of token.split(".").slice(0, 2))
		rejectAmbiguousJson(segment);
	return token;
}

function rejectAmbiguousJson(segment: string): void {
	const bytes = Buffer.from(segment, "base64url");
	if (bytes.toString("base64url") !== segment) throw new RealtimeAuthError();
	const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	const value: unknown = JSON.parse(text);
	if (
		!value ||
		Array.isArray(value) ||
		typeof value !== "object" ||
		Object.values(value).some(
			(item) => typeof item !== "string" && typeof item !== "number",
		)
	)
		throw new RealtimeAuthError();
	const names = new Set<string>();
	for (let index = 0; index < text.length; index++) {
		if (text[index] !== '"') continue;
		const start = index++;
		for (; index < text.length; index++) {
			if (text[index] === "\\") index++;
			else if (text[index] === '"') break;
		}
		let after = index + 1;
		while (/\s/.test(text[after] ?? "") && after < text.length) after++;
		if (text[after] !== ":") continue;
		const name = JSON.parse(text.slice(start, index + 1)) as string;
		if (names.has(name)) throw new RealtimeAuthError();
		names.add(name);
	}
}

/** Transport admission conveys no authority to decrypt or execute management requests. */
export async function createDeviceAuthenticator(config: RealtimeAuthConfig) {
	const verificationKey = config.insecureLocalDev
		? null
		: await importSPKI(config.publicKeyPem ?? "", "ES256");
	return async (
		origin: string | null,
		protocols: string | null,
	): Promise<DeviceAdmission> => {
		try {
			// Device management never inherits the collaboration server's anonymous dev mode.
			if (!verificationKey) throw new RealtimeAuthError();
			const token = deviceToken(protocols);
			const { payload, protectedHeader } = await jwtVerify(
				token,
				verificationKey,
				{
					algorithms: ["ES256"],
					issuer: config.issuer,
					audience: DEVICE_SIGNALING_AUDIENCE,
					clockTolerance: 5,
					maxTokenAge: `${TOKEN_LIFETIME_SECONDS}s`,
					requiredClaims: CLAIMS,
				},
			);
			if (
				protectedHeader.alg !== "ES256" ||
				protectedHeader.typ !== DEVICE_SIGNALING_JOSE_TYPE ||
				typeof protectedHeader.kid !== "string" ||
				!protectedHeader.kid ||
				protectedHeader.kid.length > 128 ||
				/[\s,]/.test(protectedHeader.kid) ||
				(config.expectedKeyId &&
					protectedHeader.kid !== config.expectedKeyId) ||
				Object.keys(protectedHeader).some(
					(key) => !["alg", "typ", "kid"].includes(key),
				) ||
				Object.keys(payload).some((key) => !CLAIMS.includes(key)) ||
				payload.iss !== config.issuer ||
				payload.aud !== DEVICE_SIGNALING_AUDIENCE ||
				payload.typ !== "device_signaling" ||
				payload.scope !== DEVICE_SIGNALING_SCOPE ||
				!deviceIdentifier(payload.device_id) ||
				!deviceIdentifier(payload.participant_id) ||
				!deviceIdentifier(payload.jti) ||
				!Number.isSafeInteger(payload.device_auth_epoch) ||
				typeof payload.device_auth_epoch !== "number" ||
				payload.device_auth_epoch < 1 ||
				!["device", "controller"].includes(String(payload.role)) ||
				typeof payload.sub !== "string" ||
				!payload.sub ||
				payload.sub.length > 256 ||
				typeof payload.iat !== "number" ||
				!Number.isSafeInteger(payload.iat) ||
				!Number.isSafeInteger(payload.nbf) ||
				typeof payload.exp !== "number" ||
				!Number.isSafeInteger(payload.exp) ||
				payload.iat < 0 ||
				payload.iat * 1000 > Date.now() + 5000 ||
				payload.nbf !== payload.iat ||
				payload.exp <= payload.iat ||
				payload.exp - payload.iat > TOKEN_LIFETIME_SECONDS ||
				payload.exp * 1000 <= Date.now()
			)
				throw new RealtimeAuthError();
			const admission: DeviceAdmission = {
				deviceId: payload.device_id,
				deviceAuthEpoch: payload.device_auth_epoch,
				participantId: payload.participant_id,
				role: payload.role as DeviceAdmission["role"],
				subject: payload.sub,
				expiresAtMs: payload.exp * 1000,
			};
			deviceInbox(admission);
			if (
				(admission.role === "device" &&
					admission.subject !== admission.deviceId) ||
				((admission.role === "controller" || origin !== null) &&
					!originIsAllowed(origin, config.allowedOrigins))
			)
				throw new RealtimeAuthError();
			return admission;
		} catch {
			throw new RealtimeAuthError();
		}
	};
}
