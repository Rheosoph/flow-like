import { createPrivateKey, generateKeyPairSync } from "node:crypto";
import {
	constants,
	closeSync,
	fchmodSync,
	fsyncSync,
	lstatSync,
	openSync,
	readFileSync,
	renameSync,
	unlinkSync,
	writeFileSync,
} from "node:fs";
import { dirname, resolve } from "node:path";

function syncDirectory(path: string) {
	const fd = openSync(path, constants.O_RDONLY);
	try {
		fsyncSync(fd);
	} finally {
		closeSync(fd);
	}
}

/** Returns only the public key. The private seed never enters command output. */
export function releaseAuthority(keyPath: string, initialize = false): string {
	const path = resolve(keyPath);
	if (initialize) {
		const pair = generateKeyPairSync("ed25519");
		const key = pair.privateKey.export({ format: "jwk" });
		const fd = openSync(
			path,
			constants.O_WRONLY |
				constants.O_CREAT |
				constants.O_EXCL |
				constants.O_NOFOLLOW,
			0o600,
		);
		try {
			writeFileSync(fd, `${key.d}\n`);
			fsyncSync(fd);
		} finally {
			closeSync(fd);
		}
		syncDirectory(dirname(path));
	}
	const stat = lstatSync(path);
	if (
		!stat.isFile() ||
		stat.isSymbolicLink() ||
		stat.size > 128 ||
		(stat.mode & 0o077) !== 0
	)
		throw new Error(
			"The release signing key must be a private regular file with mode 0600.",
		);
	const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
	let encoded: string;
	try {
		encoded = readFileSync(fd, "utf8").trim();
	} finally {
		closeSync(fd);
	}
	const seed = Buffer.from(encoded, "base64url");
	if (seed.length !== 32 || seed.toString("base64url") !== encoded)
		throw new Error(
			"The release signing key must contain a canonical base64url Ed25519 seed.",
		);
	const der = Buffer.concat([
		Buffer.from("302e020100300506032b657004220420", "hex"),
		seed,
	]);
	try {
		const key = createPrivateKey({
			key: der,
			format: "der",
			type: "pkcs8",
		}).export({ format: "jwk" });
		if (!key.x) throw new Error("The release public key could not be derived.");
		return key.x;
	} finally {
		seed.fill(0);
		der.fill(0);
	}
}

export function configureRelease(
	configPath: string,
	publicKey: string,
	manifestUrl: string,
	minimumSequence: number,
) {
	const publicBytes = Buffer.from(publicKey, "base64url");
	if (
		publicBytes.length !== 32 ||
		publicBytes.toString("base64url") !== publicKey
	)
		throw new Error(
			"The release public key must be a canonical base64url Ed25519 public key.",
		);
	const url = new URL(manifestUrl);
	if (
		url.protocol !== "https:" ||
		url.href !== manifestUrl ||
		url.username ||
		url.password ||
		url.search ||
		url.hash ||
		manifestUrl.length > 1024
	)
		throw new Error(
			"Use a canonical direct HTTPS manifest URL without credentials, query or fragment.",
		);
	if (!Number.isSafeInteger(minimumSequence) || minimumSequence < 1)
		throw new Error(
			"The minimum release sequence must be a positive safe integer.",
		);
	const path = resolve(configPath);
	const stat = lstatSync(path);
	if (!stat.isFile() || stat.isSymbolicLink())
		throw new Error("The hub config must be a regular file.");
	const config = JSON.parse(readFileSync(path, "utf8"));
	const previous = config.standalone?.release_trust;
	if (
		previous &&
		(!previous.public_keys.includes(publicKey) ||
			previous.minimum_sequence > minimumSequence)
	)
		throw new Error(
			"Refusing to replace an existing release authority or lower the minimum release sequence.",
		);
	config.standalone = {
		...config.standalone,
		release_trust: {
			manifest_url: manifestUrl,
			public_keys: previous?.public_keys ?? [publicKey],
			minimum_sequence: minimumSequence,
		},
	};
	const temporary = `${path}.${crypto.randomUUID()}.tmp`;
	const fd = openSync(
		temporary,
		constants.O_WRONLY |
			constants.O_CREAT |
			constants.O_EXCL |
			constants.O_NOFOLLOW,
		0o600,
	);
	try {
		writeFileSync(fd, `${JSON.stringify(config, null, "\t")}\n`);
		fchmodSync(fd, stat.mode & 0o777);
		fsyncSync(fd);
	} catch (error) {
		unlinkSync(temporary);
		throw error;
	} finally {
		closeSync(fd);
	}
	renameSync(temporary, path);
	syncDirectory(dirname(path));
}

if (import.meta.main) {
	const { parseArgs } = await import("node:util");
	const { values } = parseArgs({
		args: process.argv.slice(2),
		options: {
			"key-file": { type: "string" },
			init: { type: "boolean", default: false },
			config: { type: "string" },
			"manifest-url": { type: "string" },
			"minimum-sequence": { type: "string", default: "1" },
		},
	});
	if (!values["key-file"])
		throw new Error(
			"Specify --key-file; add --init only when creating a new authority.",
		);
	const publicKey = releaseAuthority(values["key-file"], values.init);
	if (values.config) {
		if (!values["manifest-url"])
			throw new Error("Specify --manifest-url when updating a hub config.");
		configureRelease(
			values.config,
			publicKey,
			values["manifest-url"],
			Number(values["minimum-sequence"]),
		);
	}
	process.stdout.write(
		`${JSON.stringify({ public_key: publicKey, config_updated: Boolean(values.config) })}\n`,
	);
}
