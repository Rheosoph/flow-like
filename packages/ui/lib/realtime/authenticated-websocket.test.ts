import { describe, expect, test } from "bun:test";
import {
	decodeJwtExpiryMs,
	prepareAuthenticatedSignaling,
	scopedSignalingUrl,
} from "./authenticated-websocket";

function fakeJwt(payload: object): string {
	const encode = (value: object) =>
		Buffer.from(JSON.stringify(value))
			.toString("base64")
			.replace(/\+/g, "-")
			.replace(/\//g, "_")
			.replace(/=+$/g, "");
	return `${encode({ alg: "ES256" })}.${encode(payload)}.c2ln`;
}

describe("authenticated realtime signaling URL", () => {
	test("creates a deterministic credential-free room scope", async () => {
		const first = await scopedSignalingUrl(
			"wss://signaling.example.com/ws",
			"app_123:board_456",
		);
		const second = await scopedSignalingUrl(
			"wss://signaling.example.com",
			"app_123:board_456",
		);
		const other = await scopedSignalingUrl(
			"wss://signaling.example.com",
			"app_123:board_other",
		);

		expect(first).toBe(second);
		expect(first).toMatch(
			/^wss:\/\/signaling\.example\.com\/ws\/session\/[A-Za-z0-9_-]{32}$/,
		);
		expect(first).not.toContain("app_123");
		expect(first).not.toContain("board_456");
		expect(other).not.toBe(first);
	});

	test("rejects URL fields that could carry or leak a credential", async () => {
		expect(
			scopedSignalingUrl(
				"wss://signaling.example.com/ws?token=secret",
				"app_123:board_456",
			),
		).rejects.toThrow();
		expect(
			scopedSignalingUrl(
				"wss://user:password@signaling.example.com/ws",
				"app_123:board_456",
			),
		).rejects.toThrow();
	});
});

describe("realtime token expiry decoding", () => {
	test("reads exp from the payload without verifying the signature", () => {
		expect(decodeJwtExpiryMs(fakeJwt({ exp: 1_700_000_000 }))).toBe(
			1_700_000_000_000,
		);
		expect(decodeJwtExpiryMs(fakeJwt({ sub: "user" }))).toBeNull();
		expect(decodeJwtExpiryMs("not-a-jwt")).toBeNull();
		expect(decodeJwtExpiryMs(fakeJwt({ exp: "soon" }))).toBeNull();
	});
});

describe("realtime token rotation", () => {
	test("rotates the registered credential for a live scope", async () => {
		const room = "app_123:board_456";
		const endpoint = "wss://rotation.example.com";
		const tokenA = fakeJwt({ exp: 1, jti: "a" });
		const tokenB = fakeJwt({ exp: 2, jti: "b" });

		const first = await prepareAuthenticatedSignaling([endpoint], room, tokenA);
		// A differing token for the same live scope is still rejected...
		expect(
			prepareAuthenticatedSignaling([endpoint], room, tokenB),
		).rejects.toThrow("another credential");

		// ...until the registration is explicitly rotated.
		first.rotate(tokenB);
		const second = await prepareAuthenticatedSignaling(
			[endpoint],
			room,
			tokenB,
		);

		second.dispose();
		first.dispose();

		// A fully released scope accepts a fresh credential again.
		const third = await prepareAuthenticatedSignaling([endpoint], room, tokenA);
		third.dispose();
	});
});
