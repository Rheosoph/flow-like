import { describe, expect, test } from "bun:test";
import { SignJWT, exportSPKI, generateKeyPair } from "jose";
import {
	REALTIME_PROTOCOL,
	REALTIME_TOKEN_PROTOCOL_PREFIX,
	createRealtimeAuthenticator,
	deriveConversationTopic,
	deriveRealtimeTopic,
	deriveTopicForClaims,
	extractRealtimeToken,
	originIsAllowed,
	parseRealtimeAuthConfig,
} from "../auth";
import {
	ConnectionRateLimiter,
	MAX_CONNECTION_MESSAGES,
	MESSAGE_BURST_CAPACITY,
	PUBLISH_BURST_CAPACITY,
	RATE_REFILL_WINDOW_MS,
} from "../limits";

const issuer = "flow-like";
const audience = "y-webrtc";
const origin = "https://app.example.com";
const keyId = "backend-es256-v1";
const desktopOrigins = [
	"tauri://localhost",
	"http://tauri.localhost",
	"https://tauri.localhost",
];

async function fixture(
	allowedOrigins = origin,
	board_format_version?: unknown,
) {
	return claimsFixture(
		{ app_id: "app_123", board_id: "board_456", board_format_version },
		allowedOrigins,
	);
}

async function claimsFixture(
	claims: Record<string, unknown>,
	allowedOrigins = origin,
) {
	const { privateKey, publicKey } = await generateKeyPair("ES256");
	const publicKeyPem = await exportSPKI(publicKey);
	const config = parseRealtimeAuthConfig({
		BACKEND_PUB: Buffer.from(publicKeyPem).toString("base64"),
		BACKEND_KID: keyId,
		REALTIME_ALLOWED_ORIGINS: allowedOrigins,
		REALTIME_JWT_ISSUER: issuer,
		REALTIME_JWT_AUDIENCE: audience,
		REALTIME_ALLOW_INSECURE_LOCAL_DEV: "false",
	});
	const now = Math.floor(Date.now() / 1000);
	const token = await new SignJWT({
		typ: "realtime",
		scope: "realtime.read",
		...claims,
	})
		.setProtectedHeader({ alg: "ES256", kid: keyId })
		.setIssuer(issuer)
		.setAudience(audience)
		.setSubject("user_789")
		.setJti("token_123")
		.setIssuedAt(now)
		.setNotBefore(now - 30)
		.setExpirationTime(now + 3 * 60 * 60)
		.sign(privateKey);
	return { config, privateKey, token };
}

describe("realtime signaling authentication", () => {
	test("validates the signed token and derives its only allowed topic", async () => {
		const { config, token } = await fixture();
		const authenticate = await createRealtimeAuthenticator(config);
		const authorization = await authenticate(
			origin,
			`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
		);

		expect(authorization.allowedTopic).toBe("app_123:board_456");
		expect(authorization.subject).toBe("user_789");
		expect(authorization.insecureLocalDev).toBeFalse();
		expect(authorization.expiresAtMs).toBeGreaterThan(Date.now());
	});

	test("isolates clients using their signed negotiated board format", async () => {
		for (const version of [1, 2, 3]) {
			const { config, token } = await fixture(origin, version);
			const authenticate = await createRealtimeAuthenticator(config);
			const authorization = await authenticate(
				origin,
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			);
			expect(authorization.allowedTopic).toBe(
				version === 1
					? "app_123:board_456"
					: `app_123:board_456:format-v${version}`,
			);
		}
	});

	test("rejects invalid signed board format claims instead of joining a legacy room", async () => {
		for (const version of [null, "2", 0, -1, 1.5, 0x1_0000_0000, []]) {
			const { config, token } = await fixture(origin, version);
			const authenticate = await createRealtimeAuthenticator(config);
			await expect(
				authenticate(
					origin,
					`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
				),
			).rejects.toThrow("Realtime authorization failed");
			expect(() =>
				deriveRealtimeTopic("app_123", "board_456", version),
			).toThrow();
		}
	});

	test("gives a conversation token only its conversation room", async () => {
		const { config, token } = await claimsFixture({
			conversation_id: "conv_123",
		});
		const authenticate = await createRealtimeAuthenticator(config);
		const authorization = await authenticate(
			origin,
			`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
		);
		expect(authorization.allowedTopic).toBe("inbox:conv_123:room-v1");
		expect(authorization.subject).toBe("user_789");
	});

	test("isolates meeting epochs from their parent conversation and old sessions", async () => {
		const roomId = "meeting-call_123-2";
		const { config, token } = await claimsFixture({ conversation_id: roomId });
		const authenticate = await createRealtimeAuthenticator(config);
		const authorization = await authenticate(
			origin,
			`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
		);
		expect(authorization.allowedTopic).toBe(`inbox:${roomId}:room-v1`);
		expect(authorization.allowedTopic).not.toBe(deriveConversationTopic("conv_123"));
		expect(authorization.allowedTopic).not.toBe(deriveConversationTopic("meeting-call_123-1"));
		expect(authorization.allowedTopic).not.toBe(deriveConversationTopic("meeting-other_call-2"));
	});

	test("rejects a token naming a conversation and a board", async () => {
		for (const extra of [
			{ app_id: "app_123", board_id: "board_456" },
			{ app_id: "app_123" },
			{ board_id: "board_456" },
			{ board_format_version: 1 },
		]) {
			const { config, token } = await claimsFixture({
				conversation_id: "conv_123",
				...extra,
			});
			const authenticate = await createRealtimeAuthenticator(config);
			await expect(
				authenticate(
					origin,
					`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
				),
			).rejects.toThrow("Realtime authorization failed");
		}
	});

	test("rejects malformed conversation ids", async () => {
		for (const conversation_id of ["", "conv:123", "a".repeat(129), 7, null]) {
			const { config, token } = await claimsFixture({ conversation_id });
			const authenticate = await createRealtimeAuthenticator(config);
			await expect(
				authenticate(
					origin,
					`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
				),
			).rejects.toThrow("Realtime authorization failed");
			expect(() => deriveConversationTopic(conversation_id)).toThrow();
		}
	});

	test("keeps conversation and board topics apart", () => {
		const conversation = deriveConversationTopic("board_456");
		for (const version of [1, 2]) {
			expect(deriveRealtimeTopic("inbox", "board_456", version)).not.toBe(
				conversation,
			);
		}
		expect(deriveTopicForClaims({ conversation_id: "board_456" })).toBe(
			conversation,
		);
	});

	test("rejects the same token from an unlisted browser origin", async () => {
		const { config, token } = await fixture();
		const authenticate = await createRealtimeAuthenticator(config);
		expect(
			authenticate(
				"https://evil.example.com",
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).rejects.toThrow("Realtime authorization failed");
	});

	test("rejects a valid signature with the wrong exact audience", async () => {
		const { config, privateKey } = await fixture();
		const now = Math.floor(Date.now() / 1000);
		const token = await new SignJWT({
			typ: "realtime",
			scope: "realtime.read",
			app_id: "app_123",
			board_id: "board_456",
		})
			.setProtectedHeader({ alg: "ES256", kid: keyId })
			.setIssuer(issuer)
			.setAudience("another-audience")
			.setSubject("user_789")
			.setJti("token_123")
			.setIssuedAt(now)
			.setNotBefore(now - 30)
			.setExpirationTime(now + 600)
			.sign(privateKey);
		const authenticate = await createRealtimeAuthenticator(config);
		expect(
			authenticate(
				origin,
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).rejects.toThrow("Realtime authorization failed");
	});

	test("accepts only the stable protocol plus one JWT protocol", async () => {
		const { token } = await fixture();
		expect(
			extractRealtimeToken(
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).toBe(token);
		expect(() => extractRealtimeToken(null)).toThrow();
		expect(() =>
			extractRealtimeToken(
				`${REALTIME_PROTOCOL}, extra, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).toThrow();
	});

	test("requires explicit local-dev opt-in and refuses it with production or Azure Redis", () => {
		expect(
			parseRealtimeAuthConfig({
				REALTIME_ALLOW_INSECURE_LOCAL_DEV: "true",
				NODE_ENV: "development",
				REDIS_AUTH_MODE: "local",
			}).insecureLocalDev,
		).toBeTrue();
		expect(() =>
			parseRealtimeAuthConfig({
				REALTIME_ALLOW_INSECURE_LOCAL_DEV: "true",
				NODE_ENV: "production",
				REDIS_AUTH_MODE: "local",
			}),
		).toThrow();
		expect(() =>
			parseRealtimeAuthConfig({
				REALTIME_ALLOW_INSECURE_LOCAL_DEV: "true",
				NODE_ENV: "development",
				REDIS_AUTH_MODE: "azure-entra",
			}),
		).toThrow();
	});

	test("normalizes origins and rejects ambiguous topic identifiers", () => {
		expect(originIsAllowed(`${origin}/`, new Set([origin]))).toBeTrue();
		expect(deriveRealtimeTopic("app_123", "board_456")).toBe(
			"app_123:board_456",
		);
		expect(() => deriveRealtimeTopic("app:123", "board_456")).toThrow();
	});

	test("accepts the exact Tauri desktop origins when configured", async () => {
		const { config, token } = await fixture(
			[origin, ...desktopOrigins].join(","),
		);
		for (const desktopOrigin of desktopOrigins) {
			expect(config.allowedOrigins.has(desktopOrigin)).toBeTrue();
			expect(originIsAllowed(desktopOrigin, config.allowedOrigins)).toBeTrue();
		}

		const authenticate = await createRealtimeAuthenticator(config);
		const authorization = await authenticate(
			"tauri://localhost",
			`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
		);
		expect(authorization.allowedTopic).toBe("app_123:board_456");
		expect(authorization.subject).toBe("user_789");
	});

	test("rejects desktop origins when they are not configured", async () => {
		const { config, token } = await fixture();
		for (const desktopOrigin of desktopOrigins) {
			expect(originIsAllowed(desktopOrigin, config.allowedOrigins)).toBeFalse();
		}

		const authenticate = await createRealtimeAuthenticator(config);
		expect(
			authenticate(
				"tauri://localhost",
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).rejects.toThrow("Realtime authorization failed");
	});

	test("still rejects a missing Origin and non-literal custom schemes", async () => {
		const { config, token } = await fixture(
			[origin, ...desktopOrigins].join(","),
		);
		const authenticate = await createRealtimeAuthenticator(config);
		expect(
			authenticate(
				null,
				`${REALTIME_PROTOCOL}, ${REALTIME_TOKEN_PROTOCOL_PREFIX}${token}`,
			),
		).rejects.toThrow("Realtime authorization failed");
		expect(originIsAllowed("null", config.allowedOrigins)).toBeFalse();
		expect(originIsAllowed("tauri://evil", config.allowedOrigins)).toBeFalse();

		expect(() =>
			parseRealtimeAuthConfig({
				BACKEND_PUB: config.publicKeyPem
					? Buffer.from(config.publicKeyPem).toString("base64")
					: "",
				REALTIME_ALLOWED_ORIGINS: "tauri://evil",
			}),
		).toThrow();
		expect(() =>
			parseRealtimeAuthConfig({
				BACKEND_PUB: config.publicKeyPem
					? Buffer.from(config.publicKeyPem).toString("base64")
					: "",
				REALTIME_ALLOWED_ORIGINS: "file://localhost",
			}),
		).toThrow();
	});
});

describe("per-connection signaling limits", () => {
	test("bounds message and publish bursts and refills over time", () => {
		const startedAt = 1_000;
		const messages = new ConnectionRateLimiter(startedAt);
		for (let index = 0; index < MESSAGE_BURST_CAPACITY; index++) {
			expect(messages.consume(false, startedAt)).toBeTrue();
		}
		expect(messages.consume(false, startedAt)).toBeFalse();
		expect(
			messages.consume(false, startedAt + RATE_REFILL_WINDOW_MS),
		).toBeTrue();

		const publishes = new ConnectionRateLimiter(startedAt);
		for (let index = 0; index < PUBLISH_BURST_CAPACITY; index++) {
			expect(publishes.consume(true, startedAt)).toBeTrue();
		}
		expect(publishes.consume(true, startedAt)).toBeFalse();
	});

	test("enforces a hard lifetime message ceiling", () => {
		const limiter = new ConnectionRateLimiter(0);
		for (let index = 0; index < MAX_CONNECTION_MESSAGES; index++) {
			expect(limiter.consume(false, index * RATE_REFILL_WINDOW_MS)).toBeTrue();
		}
		expect(
			limiter.consume(
				false,
				(MAX_CONNECTION_MESSAGES + 1) * RATE_REFILL_WINDOW_MS,
			),
		).toBeFalse();
	});
});
