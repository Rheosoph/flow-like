import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

function decode(value: string): Buffer {
	const text = value.trim();
	const bytes = Buffer.from(text, "base64");
	if (!text || bytes.toString("base64") !== text) {
		throw new Error("Invalid updater signature or public key encoding");
	}
	return bytes;
}

/** Authenticate the minisign trusted comment before publishing its release version. */
export function verifyUpdaterSignature(
	encodedSignature: string,
	encodedPublicKey: string,
	version: string,
	payload?: Uint8Array,
): void {
	const keyLines = decode(encodedPublicKey)
		.toString("utf8")
		.trimEnd()
		.split(/\r?\n/);
	const lines = decode(encodedSignature)
		.toString("utf8")
		.trimEnd()
		.split(/\r?\n/);
	if (
		keyLines.length !== 2 ||
		lines.length !== 4 ||
		!lines[2].startsWith("trusted comment: ")
	) {
		throw new Error("Invalid minisign document");
	}
	const publicKey = decode(keyLines[1]);
	const signature = decode(lines[1]);
	const globalSignature = decode(lines[3]);
	if (
		publicKey.length !== 42 ||
		publicKey.subarray(0, 2).toString() !== "Ed" ||
		signature.length !== 74 ||
		!["Ed", "ED"].includes(signature.subarray(0, 2).toString()) ||
		globalSignature.length !== 64
	) {
		throw new Error("Unsupported minisign key or signature format");
	}
	if (!publicKey.subarray(2, 10).equals(signature.subarray(2, 10))) {
		throw new Error(
			"Updater signature does not match the configured public key",
		);
	}
	const key = createPublicKey({
		key: Buffer.concat([
			Buffer.from("302a300506032b6570032100", "hex"),
			publicKey.subarray(10),
		]),
		format: "der",
		type: "spki",
	});
	const comment = lines[2].slice("trusted comment: ".length);
	if (
		!verify(
			null,
			Buffer.concat([signature.subarray(10), Buffer.from(comment)]),
			key,
			globalSignature,
		)
	) {
		throw new Error("Updater signature trusted comment authentication failed");
	}
	const versions = comment
		.split("\t")
		.filter((field) => field.startsWith("version:"));
	if (versions.length !== 1 || versions[0] !== `version:${version}`) {
		throw new Error(
			"Updater signed version does not match the release version",
		);
	}
	// Finalization authenticates metadata without downloading every installer.
	// The updater verifies each payload before installation; the signer probe also checks it here.
	if (payload) {
		const data =
			signature.subarray(0, 2).toString() === "ED"
				? createHash("blake2b512").update(payload).digest()
				: payload;
		if (!verify(null, data, key, signature.subarray(10))) {
			throw new Error("Updater payload signature verification failed");
		}
	}
}

if (import.meta.main) {
	const [configPath, signaturePath, version] = process.argv.slice(2);
	if (!configPath || !signaturePath || !version) {
		throw new Error(
			"Usage: verify-updater-signature.ts CONFIG SIGNATURE VERSION",
		);
	}
	const config = JSON.parse(readFileSync(configPath, "utf8"));
	if (config.version !== version) {
		throw new Error("Updater manifest version must match tauri.conf.json");
	}
	verifyUpdaterSignature(
		readFileSync(signaturePath, "utf8"),
		config.plugins.updater.pubkey,
		version,
	);
}
