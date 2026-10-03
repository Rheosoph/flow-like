import { sha256 } from "@noble/hashes/sha2";
import { base64url, unbase64url } from "./crypto";
import type { DeviceReceipt } from "./types";

const DOMAIN = new TextEncoder().encode("flow-like-device-identity-v1");
const KEY_BYTES = 32;
const FINGERPRINT_CHARS = 16;

function publicKey(name: string, bytes: Uint8Array | number[]): Uint8Array {
	if (
		bytes.length !== KEY_BYTES ||
		!Array.from(bytes).every(
			(byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255,
		)
	)
		throw new Error(
			`The device identity's ${name} is not a ${KEY_BYTES}-byte public key.`,
		);
	return Uint8Array.from(bytes);
}

/** BG21: the same 16 characters `flow-like-standalone status` prints on the device. */
export function identityFingerprint(
	identity: DeviceReceipt["identity"],
): string {
	const hash = sha256.create().update(DOMAIN);
	hash.update(
		publicKey("auth key", unbase64url(identity.auth_key.x, KEY_BYTES)),
	);
	hash.update(
		publicKey(
			"telemetry key",
			unbase64url(identity.telemetry_key.x, KEY_BYTES),
		),
	);
	hash.update(publicKey("management key", identity.management_key));
	return base64url(hash.digest()).slice(0, FINGERPRINT_CHARS);
}

/** Four groups of four, as the device prints it: "OYLH wejQ rTGz ZJCW". */
export function groupFingerprint(fingerprint: string): string {
	return fingerprint.match(/.{1,4}/gu)?.join(" ") ?? fingerprint;
}
