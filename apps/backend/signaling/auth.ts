import { importSPKI, jwtVerify } from "jose";

export const REALTIME_PROTOCOL = "flowlike.realtime.v1";
export const REALTIME_TOKEN_PROTOCOL_PREFIX = "flowlike.jwt.";
export const DEFAULT_REALTIME_ISSUER = "flow-like";
export const DEFAULT_REALTIME_AUDIENCE = "y-webrtc";
export const REALTIME_SCOPE = "realtime.read";

const MAX_TOKEN_BYTES = 4096;
const MAX_TOKEN_LIFETIME_SECONDS = 3 * 60 * 60;
const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const JWT_PATTERN = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/;

// Exact origins sent by the Tauri desktop WebView: tauri://localhost on
// macOS/Linux, http(s)://tauri.localhost on Windows. These are the only
// non-standard entries REALTIME_ALLOWED_ORIGINS accepts — no wildcards and no
// blanket non-http(s) scheme support.
export const DESKTOP_APP_ORIGINS: ReadonlySet<string> = new Set([
	"tauri://localhost",
	"http://tauri.localhost",
	"https://tauri.localhost",
]);

type Environment = Record<string, string | undefined>;

export type RealtimeAuthConfig = {
	insecureLocalDev: boolean;
	issuer: string;
	audience: string;
	allowedOrigins: ReadonlySet<string>;
	publicKeyPem?: string;
	expectedKeyId?: string;
};

export type RealtimeAuthorization = {
	allowedTopic: string | null;
	expiresAtMs: number | null;
	subject: string | null;
	insecureLocalDev: boolean;
};

export class RealtimeAuthError extends Error {
	constructor(message = "Realtime authorization failed") {
		super(message);
		this.name = "RealtimeAuthError";
	}
}

function parseExactBoolean(value: string | undefined, name: string): boolean {
	if (value === undefined || value.trim() === "") return false;
	if (value === "true") return true;
	if (value === "false") return false;
	throw new Error(`${name} must be exactly true or false`);
}

function parseOrigin(value: string, name: string): string {
	if (DESKTOP_APP_ORIGINS.has(value)) return value;
	let parsed: URL;
	try {
		parsed = new URL(value);
	} catch {
		throw new Error(`${name} contains an invalid origin`);
	}
	if (
		(parsed.protocol !== "https:" && parsed.protocol !== "http:") ||
		parsed.username ||
		parsed.password ||
		parsed.pathname !== "/" ||
		parsed.search ||
		parsed.hash
	) {
		throw new Error(
			`${name} must contain exact HTTP(S) origins or the Tauri desktop origins only`,
		);
	}
	return parsed.origin;
}

function parseAllowedOrigins(raw: string | undefined): ReadonlySet<string> {
	const values = (raw || "")
		.split(",")
		.map((value) => value.trim())
		.filter(Boolean);
	if (values.length === 0) {
		throw new Error(
			"REALTIME_ALLOWED_ORIGINS is required in authenticated mode",
		);
	}
	if (values.length > 20) {
		throw new Error("REALTIME_ALLOWED_ORIGINS may contain at most 20 origins");
	}
	const origins = new Set(
		values.map((value) => parseOrigin(value, "REALTIME_ALLOWED_ORIGINS")),
	);
	if (origins.size !== values.length) {
		throw new Error("REALTIME_ALLOWED_ORIGINS must not contain duplicates");
	}
	return origins;
}

function decodePublicKey(encoded: string | undefined): string {
	if (!encoded?.trim()) {
		throw new Error("BACKEND_PUB is required in authenticated mode");
	}
	const compact = encoded.trim();
	if (
		compact.length > 16_384 ||
		compact.length % 4 !== 0 ||
		!/^[A-Za-z0-9+/]+={0,2}$/.test(compact)
	) {
		throw new Error("BACKEND_PUB must be a bounded standard-base64 value");
	}
	const pem = Buffer.from(compact, "base64").toString("utf8");
	if (
		!pem.startsWith("-----BEGIN PUBLIC KEY-----\n") ||
		!pem.trimEnd().endsWith("-----END PUBLIC KEY-----") ||
		pem.includes("PRIVATE KEY") ||
		pem.includes("\0")
	) {
		throw new Error("BACKEND_PUB must encode one PEM public key");
	}
	return pem;
}

export function parseRealtimeAuthConfig(
	env: Environment = process.env,
): RealtimeAuthConfig {
	const insecureLocalDev = parseExactBoolean(
		env.REALTIME_ALLOW_INSECURE_LOCAL_DEV,
		"REALTIME_ALLOW_INSECURE_LOCAL_DEV",
	);
	if (insecureLocalDev) {
		if ((env.NODE_ENV || "").trim().toLowerCase() === "production") {
			throw new Error(
				"REALTIME_ALLOW_INSECURE_LOCAL_DEV cannot be enabled in production",
			);
		}
		if ((env.REDIS_AUTH_MODE || "local").trim().toLowerCase() !== "local") {
			throw new Error(
				"REALTIME_ALLOW_INSECURE_LOCAL_DEV requires REDIS_AUTH_MODE=local",
			);
		}
		return {
			insecureLocalDev: true,
			issuer: DEFAULT_REALTIME_ISSUER,
			audience: DEFAULT_REALTIME_AUDIENCE,
			allowedOrigins: new Set(),
		};
	}

	const issuer = (env.REALTIME_JWT_ISSUER || DEFAULT_REALTIME_ISSUER).trim();
	const audience = (
		env.REALTIME_JWT_AUDIENCE || DEFAULT_REALTIME_AUDIENCE
	).trim();
	if (!issuer || issuer.length > 128 || !audience || audience.length > 128) {
		throw new Error("Realtime JWT issuer and audience must be bounded values");
	}
	const expectedKeyId = env.BACKEND_KID?.trim() || undefined;
	if (
		expectedKeyId &&
		(expectedKeyId.length > 128 || /[\s,]/.test(expectedKeyId))
	) {
		throw new Error("BACKEND_KID must be a bounded token value");
	}

	return {
		insecureLocalDev: false,
		issuer,
		audience,
		allowedOrigins: parseAllowedOrigins(env.REALTIME_ALLOWED_ORIGINS),
		publicKeyPem: decodePublicKey(env.BACKEND_PUB),
		expectedKeyId,
	};
}

export function originIsAllowed(
	originHeader: string | null,
	allowedOrigins: ReadonlySet<string>,
): boolean {
	if (!originHeader || originHeader.length > 2048) return false;
	try {
		return allowedOrigins.has(parseOrigin(originHeader, "Origin"));
	} catch {
		return false;
	}
}

export function extractRealtimeToken(protocolHeader: string | null): string {
	if (!protocolHeader || protocolHeader.length > MAX_TOKEN_BYTES + 256) {
		throw new RealtimeAuthError();
	}
	const protocols = protocolHeader
		.split(",")
		.map((protocol) => protocol.trim())
		.filter(Boolean);
	if (protocols.length !== 2 || new Set(protocols).size !== protocols.length) {
		throw new RealtimeAuthError();
	}
	if (!protocols.includes(REALTIME_PROTOCOL)) {
		throw new RealtimeAuthError();
	}
	const authProtocol = protocols.find((protocol) =>
		protocol.startsWith(REALTIME_TOKEN_PROTOCOL_PREFIX),
	);
	if (!authProtocol) throw new RealtimeAuthError();
	const token = authProtocol.slice(REALTIME_TOKEN_PROTOCOL_PREFIX.length);
	if (
		!token ||
		Buffer.byteLength(token, "utf8") > MAX_TOKEN_BYTES ||
		!JWT_PATTERN.test(token)
	) {
		throw new RealtimeAuthError();
	}
	return token;
}

export function deriveRealtimeTopic(appId: unknown, boardId: unknown): string {
	if (
		typeof appId !== "string" ||
		typeof boardId !== "string" ||
		!ID_PATTERN.test(appId) ||
		!ID_PATTERN.test(boardId)
	) {
		throw new RealtimeAuthError();
	}
	return `${appId}:${boardId}`;
}

function requiredBoundedString(value: unknown, maximum: number): string {
	if (
		typeof value !== "string" ||
		value.length === 0 ||
		value.length > maximum
	) {
		throw new RealtimeAuthError();
	}
	return value;
}

function requiredInteger(value: unknown): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value)) {
		throw new RealtimeAuthError();
	}
	return value;
}

export async function createRealtimeAuthenticator(config: RealtimeAuthConfig) {
	let verificationKey: CryptoKey | Uint8Array | null = null;
	if (!config.insecureLocalDev) {
		verificationKey = await importSPKI(config.publicKeyPem!, "ES256");
	}

	return async function authorizeUpgrade(
		originHeader: string | null,
		protocolHeader: string | null,
	): Promise<RealtimeAuthorization> {
		if (config.insecureLocalDev) {
			return {
				allowedTopic: null,
				expiresAtMs: null,
				subject: null,
				insecureLocalDev: true,
			};
		}
		if (!originIsAllowed(originHeader, config.allowedOrigins)) {
			throw new RealtimeAuthError();
		}

		const token = extractRealtimeToken(protocolHeader);
		try {
			const { payload, protectedHeader } = await jwtVerify(
				token,
				verificationKey!,
				{
					algorithms: ["ES256"],
					issuer: config.issuer,
					audience: config.audience,
					clockTolerance: 5,
					maxTokenAge: `${MAX_TOKEN_LIFETIME_SECONDS}s`,
					requiredClaims: ["iss", "aud", "sub", "iat", "nbf", "exp", "jti"],
				},
			);
			if (
				protectedHeader.alg !== "ES256" ||
				!protectedHeader.kid ||
				protectedHeader.kid.length > 128 ||
				(config.expectedKeyId && protectedHeader.kid !== config.expectedKeyId)
			) {
				throw new RealtimeAuthError();
			}
			if (
				payload.iss !== config.issuer ||
				payload.aud !== config.audience ||
				payload.typ !== "realtime" ||
				payload.scope !== REALTIME_SCOPE
			) {
				throw new RealtimeAuthError();
			}
			const issuedAt = requiredInteger(payload.iat);
			const notBefore = requiredInteger(payload.nbf);
			const expiresAt = requiredInteger(payload.exp);
			if (
				expiresAt <= issuedAt ||
				expiresAt - issuedAt > MAX_TOKEN_LIFETIME_SECONDS ||
				notBefore > issuedAt ||
				issuedAt - notBefore > 60
			) {
				throw new RealtimeAuthError();
			}
			const subject = requiredBoundedString(payload.sub, 256);
			requiredBoundedString(payload.jti, 256);
			return {
				allowedTopic: deriveRealtimeTopic(payload.app_id, payload.board_id),
				expiresAtMs: expiresAt * 1000,
				subject,
				insecureLocalDev: false,
			};
		} catch {
			throw new RealtimeAuthError();
		}
	};
}
