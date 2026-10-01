import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { groupFingerprint, identityFingerprint } from "./fingerprint";
import type { DeviceReceipt } from "./types";

const vector = JSON.parse(
	readFileSync(
		join(
			import.meta.dir,
			"../../../device-protocol/fixtures/identity-fingerprint.json",
		),
		"utf8",
	),
) as { identity: DeviceReceipt["identity"]; fingerprint: string };

test("the fingerprint matches the protocol's shared vector", () => {
	expect(vector.fingerprint).toMatch(/^[A-Za-z0-9_-]{16}$/u);
	expect(identityFingerprint(vector.identity)).toBe(vector.fingerprint);
	expect(groupFingerprint(vector.fingerprint)).toBe(
		[0, 4, 8, 12].map((at) => vector.fingerprint.slice(at, at + 4)).join(" "),
	);
	expect(groupFingerprint("OYLHwejQrTGzZJCW")).toBe("OYLH wejQ rTGz ZJCW");
});

test("every key changes the fingerprint and malformed keys are rejected", () => {
	const { identity } = vector;
	const swapped = {
		...identity,
		auth_key: identity.telemetry_key,
		telemetry_key: identity.auth_key,
	};
	const management = [...identity.management_key];
	management[31] ^= 1;
	for (const changed of [swapped, { ...identity, management_key: management }])
		expect(identityFingerprint(changed)).not.toBe(vector.fingerprint);
	for (const broken of [
		{ ...identity, management_key: identity.management_key.slice(1) },
		{ ...identity, management_key: [...management.slice(1), 256] },
		{ ...identity, auth_key: { ...identity.auth_key, x: "c2hvcnQ" } },
	])
		expect(() => identityFingerprint(broken)).toThrow("32-byte public key");
	expect(() =>
		identityFingerprint({
			...identity,
			telemetry_key: { ...identity.telemetry_key, x: "not base64!" },
		}),
	).toThrow("Invalid management encoding");
});
