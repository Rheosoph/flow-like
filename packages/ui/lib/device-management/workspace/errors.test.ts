import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	ConnectError,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "../transport";
import { DeviceTunnelError } from "../tunnel";
import {
	CONNECTION_STEPS,
	DEVICE_ERROR_CODES,
	LiveCallError,
	classifyDeviceError,
	fleetErrorCode,
	isFatalLiveError,
	isKnownTransportMessage,
	keyErrorCode,
	keyErrorStep,
	liveErrorCode,
	rejectionCode,
	toLiveError,
} from "./errors";

const SENTENCE = /"([A-Z][^"\n]{8,}\.)"/gu;

function sentences(file: string): string[] {
	const source = readFileSync(join(import.meta.dir, "..", file), "utf8");
	return [...new Set([...source.matchAll(SENTENCE)].map((match) => match[1]))];
}

describe("transport error filing", () => {
	test("every sentence the transport throws maps to one of the seven steps and a code", () => {
		const thrown = sentences("transport.ts");
		expect(thrown.length).toBeGreaterThan(25);
		for (const message of thrown) {
			expect(isKnownTransportMessage(message)).toBe(true);
			const failure = classifyDeviceError(new Error(message));
			expect(CONNECTION_STEPS).toContain(failure.step);
			expect(DEVICE_ERROR_CODES).toContain(failure.code);
			expect(failure.code).not.toBe("connection_failed");
			expect(failure.message).toBe(message);
		}
	});

	test("typed connect errors keep their step, code and detail", () => {
		expect(
			classifyDeviceError(
				new ConnectError("getting_pass", "http", "[HTTP_500] boom", {
					status: 500,
				}),
			),
		).toEqual({
			step: "getting_pass",
			code: "http_error",
			message: "[HTTP_500] boom",
			status: 500,
			url: undefined,
		});
		expect(
			classifyDeviceError(
				new ConnectError(
					"reaching_device",
					"relay_unreachable",
					"Device signaling could not be reached.",
					{ url: "wss://hub/ws/devices" },
				),
			).url,
		).toBe("wss://hub/ws/devices");
	});

	test("tunnel closures, timeouts and expiry keep their cause", () => {
		for (const [cause, code] of [
			["connection_closed", "session_closed"],
			["heartbeat_timeout", "timeout"],
			["renewal_timeout", "timeout"],
			["open_timeout", "timeout"],
			["expired", "expired"],
			["cancelled", "cancelled"],
		]) {
			expect(
				classifyDeviceError(new DeviceTunnelError(cause, "detail")),
			).toEqual({
				step: "reading_services",
				code,
				message: "detail",
			});
		}
		expect(
			classifyDeviceError(new DeviceTunnelError("future_code", "detail")).code,
		).toBe("connection_failed");
	});

	test("request outcomes and unknown errors", () => {
		expect(
			classifyDeviceError(new ManagementRequestNotSentError("not sent")).code,
		).toBe("not_sent");
		expect(classifyDeviceError(new ManagementUnconfirmedError("op")).code).toBe(
			"no_reply",
		);
		expect(
			classifyDeviceError(
				new Error(
					"This controller cannot read device placement status. Grant missing",
				),
			),
		).toMatchObject({ step: "reading_services", code: "rejected" });
		expect(classifyDeviceError("odd", "securing")).toEqual({
			step: "securing",
			code: "connection_failed",
			message: "odd",
		});
	});

	test("live call errors carry the session cause", () => {
		const error = new LiveCallError("device_unreachable", "offline", {
			step: "reaching_device",
			code: "relay_unreachable",
			url: "wss://x",
		});
		expect(classifyDeviceError(error)).toMatchObject({
			step: "reaching_device",
			code: "relay_unreachable",
			url: "wss://x",
		});
		expect(
			classifyDeviceError(new LiveCallError("keys_locked", "locked")),
		).toMatchObject({ step: "reading_services", code: "keys_locked" });
		expect(toLiveError(error)).toEqual(error.liveError);
	});
});

describe("live error contract", () => {
	test("connect errors become LiveError causes; cancellation is not an error", () => {
		expect(
			toLiveError(
				new ConnectError("getting_pass", "access_expired", "x", {
					status: 401,
				}),
			),
		).toEqual({ step: "getting_pass", code: "access_expired", status: 401 });
		expect(
			toLiveError(
				new ConnectError("securing", "identity_confirmation_failed", "x"),
			),
		).toEqual({ step: "securing", code: "identity_confirmation_failed" });
		expect(
			toLiveError(new ConnectError("trying_direct", "handshake_failed", "x")),
		).toEqual({ step: "securing", code: "handshake_failed" });
		expect(
			toLiveError(new ConnectError("getting_pass", "cancelled", "x")),
		).toBeUndefined();
		expect(toLiveError(new Error("Management connection timed out."))).toEqual({
			step: "session",
			code: "timeout",
		});
		expect(
			toLiveError(
				new Error("Management session expired. Reconnect before continuing."),
			),
		).toEqual({ step: "session", code: "expired" });
		expect(toLiveError(new Error("other"))).toEqual({
			step: "session",
			code: "closed",
		});
	});

	test("only ended access, re-enrolment and identity failures are fatal", () => {
		expect(
			isFatalLiveError({ step: "getting_pass", code: "access_expired" }),
		).toBe(true);
		expect(
			isFatalLiveError({ step: "getting_pass", code: "epoch_mismatch" }),
		).toBe(true);
		expect(
			isFatalLiveError({
				step: "securing",
				code: "identity_confirmation_failed",
			}),
		).toBe(true);
		expect(
			isFatalLiveError({ step: "securing", code: "handshake_failed" }),
		).toBe(false);
		expect(isFatalLiveError({ step: "session", code: "closed" })).toBe(false);
		expect(
			isFatalLiveError({ step: "reaching_device", code: "relay_unreachable" }),
		).toBe(false);
	});

	test("live causes map to display codes", () => {
		expect(liveErrorCode({ step: "session", code: "closed" })).toBe(
			"session_closed",
		);
		expect(liveErrorCode({ step: "getting_pass", code: "http" })).toBe(
			"http_error",
		);
		expect(
			liveErrorCode({
				step: "reading_services",
				code: "rejected",
				rejection: { code: "limit", error: "full", retryable: false },
			}),
		).toBe("rejected_limit");
	});
});

describe("other families", () => {
	test("rejection, key and fleet codes", () => {
		expect(rejectionCode({ code: "host_policy" })).toBe("rejected_host_policy");
		expect(rejectionCode({ code: "something_new" })).toBe("rejected");
		expect(rejectionCode()).toBe("rejected");
		expect(keyErrorCode({ code: "storage" })).toBe("storage_error");
		expect(keyErrorCode({ code: "wrong_password" })).toBe("wrong_password");
		expect(keyErrorStep({ code: "identity_mismatch" })).toBe(
			"checking_identity",
		);
		expect(keyErrorStep({ code: "no_vault" })).toBe("unlocking_keys");
		expect(keyErrorCode({ code: "hub" })).toBe("http_error");
		expect(keyErrorStep({ code: "hub" })).toBe("checking_identity");
		expect(fleetErrorCode({ kind: "integrity" })).toBe("fleet_integrity");
	});

	test("codes are unique", () => {
		expect(new Set(DEVICE_ERROR_CODES).size).toBe(DEVICE_ERROR_CODES.length);
	});
});
