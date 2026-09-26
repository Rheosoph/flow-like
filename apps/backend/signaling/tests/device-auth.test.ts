import { describe, expect, test } from "bun:test";
import { CompactSign, SignJWT, exportSPKI, generateKeyPair } from "jose";
import {
	REALTIME_PROTOCOL,
	createRealtimeAuthenticator,
	parseRealtimeAuthConfig,
} from "../auth";
import { createDeviceAuthenticator } from "../device-auth";
import {
	DEVICE_SIGNALING_AUDIENCE,
	DEVICE_SIGNALING_JOSE_TYPE,
	DEVICE_SIGNALING_PROTOCOL,
	DEVICE_SIGNALING_SCOPE,
	type DeviceAdmission,
	MAX_DEVICE_PAYLOAD_BYTES,
	deviceInbox,
	parseDeviceFanout,
	relayDeviceFrame,
} from "../device-frames";
import { fanoutIsHealthy } from "../redis";

const origin = "https://app.example.com";
const keyId = "backend-es256-v1";
const keys = await generateKeyPair("ES256");
const config = parseRealtimeAuthConfig({
	BACKEND_PUB: Buffer.from(await exportSPKI(keys.publicKey)).toString("base64"),
	BACKEND_KID: keyId,
	REALTIME_ALLOWED_ORIGINS: origin,
});
const authenticate = await createDeviceAuthenticator(config);
const headers = (token: string) =>
	`${DEVICE_SIGNALING_PROTOCOL}, flowlike.jwt.${token}`;
function claims(overrides: Record<string, unknown> = {}) {
	const now = Math.floor(Date.now() / 1000);
	return {
		iss: "flow-like",
		aud: DEVICE_SIGNALING_AUDIENCE,
		sub: "owner",
		typ: "device_signaling",
		scope: DEVICE_SIGNALING_SCOPE,
		device_id: "device",
		device_auth_epoch: 2,
		participant_id: "controller-one",
		role: "controller",
		iat: now,
		nbf: now,
		exp: now + 300,
		jti: crypto.randomUUID(),
		...overrides,
	};
}
async function token(
	overrides: Record<string, unknown> = {},
	header: Record<string, unknown> = {},
) {
	return new SignJWT(claims(overrides))
		.setProtectedHeader({
			alg: "ES256",
			kid: keyId,
			typ: DEVICE_SIGNALING_JOSE_TYPE,
			...header,
		})
		.sign(keys.privateKey);
}

describe("device transport admission", () => {
	test("binds a controller to a device epoch and requires an allowed browser origin", async () => {
		const jwt = await token();
		const admitted = await authenticate(origin, headers(jwt));
		expect(deviceInbox(admitted)).toBe(
			"device-mgmt:v1:device:epoch:2:controller:controller-one",
		);
		await expect(authenticate(null, headers(jwt))).rejects.toThrow();
		await expect(
			authenticate("https://evil.example", headers(jwt)),
		).rejects.toThrow();
		await expect(
			authenticate(origin, `${REALTIME_PROTOCOL}, flowlike.jwt.${jwt}`),
		).rejects.toThrow();
		const ordinary = await createRealtimeAuthenticator(config);
		await expect(
			ordinary(origin, `${REALTIME_PROTOCOL}, flowlike.jwt.${jwt}`),
		).rejects.toThrow();
	});
	test("permits missing Origin only for a device bound to its own identity", async () => {
		const jwt = await token({
			role: "device",
			sub: "device",
			participant_id: "device",
		});
		expect((await authenticate(null, headers(jwt))).role).toBe("device");
		await expect(
			authenticate("https://evil.example", headers(jwt)),
		).rejects.toThrow();
		for (const overrides of [{ sub: "owner" }, { participant_id: "other" }])
			await expect(
				authenticate(
					null,
					headers(
						await token({
							role: "device",
							sub: "device",
							participant_id: "device",
							...overrides,
						}),
					),
				),
			).rejects.toThrow();
	});
	test("rejects wrong profiles, excess claims, ambiguous signatures, and invalid lifetimes", async () => {
		const now = Math.floor(Date.now() / 1000);
		for (const override of [
			{ typ: "device_session" },
			{ aud: "y-webrtc" },
			{ scope: "device:manage" },
			{ role: "admin" },
			{ device_id: "a:b" },
			{ device_auth_epoch: 0 },
			{ device_auth_epoch: 1.5 },
			{ app_id: "unrelated" },
			{ iat: now - 1, nbf: now - 1, exp: now + 300 },
			{ nbf: now - 1 },
			{ iat: now + 60, nbf: now + 60, exp: now + 120 },
			{ iat: now - 60, nbf: now - 60, exp: now },
		])
			await expect(
				authenticate(origin, headers(await token(override))),
			).rejects.toThrow();
		for (const header of [
			{ typ: "JWT" },
			{ kid: "wrong" },
			{ jku: "https://evil.example/key" },
		])
			await expect(
				authenticate(origin, headers(await token({}, header))),
			).rejects.toThrow();
		const duplicate = `${JSON.stringify(claims()).slice(0, -1)},"scope":"${DEVICE_SIGNALING_SCOPE}"}`;
		const signed = await new CompactSign(new TextEncoder().encode(duplicate))
			.setProtectedHeader({
				alg: "ES256",
				kid: keyId,
				typ: DEVICE_SIGNALING_JOSE_TYPE,
			})
			.sign(keys.privateKey);
		await expect(authenticate(origin, headers(signed))).rejects.toThrow();
		const anonymous = await createDeviceAuthenticator(
			parseRealtimeAuthConfig({ REALTIME_ALLOW_INSECURE_LOCAL_DEV: "true" }),
		);
		await expect(anonymous(null, null)).rejects.toThrow();
	});
});

const admission: DeviceAdmission = {
	deviceId: "device",
	deviceAuthEpoch: 2,
	participantId: "controller",
	role: "controller",
	subject: "owner",
	expiresAtMs: 2000,
};
const frame = {
	type: "frame",
	to: "device",
	channel: "noise",
	payload: Buffer.from("opaque Noise frame").toString("base64url"),
};
describe("opaque device routing and readiness", () => {
	test("stamps authenticated sender and permits only opposite-role delivery within one epoch", () => {
		const routed = relayDeviceFrame(admission, frame);
		expect(routed.topic).toBe("device-mgmt:v1:device:epoch:2:device:device");
		expect(routed.frame.from).toBe("controller");
		expect(() =>
			relayDeviceFrame(admission, { ...frame, to: "other-device" }),
		).toThrow();
		expect(() =>
			relayDeviceFrame(admission, { ...frame, from: "forged" }),
		).toThrow();
		const reply = relayDeviceFrame(
			{ ...admission, role: "device", participantId: "device" },
			{ ...frame, to: "controller" },
		);
		expect(reply.topic).toBe(deviceInbox(admission));
		expect(reply.frame.from_role).toBe("device");
		expect(deviceInbox({ ...admission, deviceAuthEpoch: 3 })).not.toBe(
			deviceInbox(admission),
		);
	});
	test("bounds canonical opaque payloads and rejects stale/malformed fanout envelopes", () => {
		expect(() =>
			relayDeviceFrame(admission, {
				...frame,
				payload: Buffer.alloc(MAX_DEVICE_PAYLOAD_BYTES).toString("base64url"),
			}),
		).not.toThrow();
		for (const payload of [
			"",
			"YQ=",
			"Y",
			Buffer.alloc(MAX_DEVICE_PAYLOAD_BYTES + 1).toString("base64url"),
		])
			expect(() =>
				relayDeviceFrame(admission, { ...frame, payload }),
			).toThrow();
		const envelope = {
			type: "device-frame",
			device_id: "device",
			device_auth_epoch: 2,
			expires_at_ms: 2000,
			_origin: "replica-one",
			frame: relayDeviceFrame(admission, frame).frame,
		};
		expect(parseDeviceFanout(JSON.stringify(envelope), 1000).topic).toBe(
			"device-mgmt:v1:device:epoch:2:device:device",
		);
		expect(() => parseDeviceFanout(JSON.stringify(envelope), 2000)).toThrow();
		expect(() =>
			parseDeviceFanout(
				JSON.stringify({ ...envelope, topic: "attacker" }),
				1000,
			),
		).toThrow();
	});
	test("requires live replica clients and a recent subscription heartbeat", () => {
		const snapshot = {
			mode: "redis" as const,
			publisherReady: true,
			presenceReady: true,
			subscriberReady: true,
			heartbeatAckMs: 1000,
		};
		expect(fanoutIsHealthy(snapshot, 31000)).toBe(true);
		expect(fanoutIsHealthy(snapshot, 31001)).toBe(false);
		for (const key of [
			"publisherReady",
			"presenceReady",
			"subscriberReady",
		] as const)
			expect(fanoutIsHealthy({ ...snapshot, [key]: false }, 1001)).toBe(false);
		expect(fanoutIsHealthy({ ...snapshot, heartbeatAckMs: 0 }, 1001)).toBe(
			false,
		);
		expect(
			fanoutIsHealthy(
				{ ...snapshot, mode: "local", publisherReady: false },
				99999,
			),
		).toBe(true);
	});
});
