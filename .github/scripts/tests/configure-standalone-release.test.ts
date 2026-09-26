import { expect, test } from "bun:test";
import {
	chmodSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	statSync,
	symlinkSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	configureRelease,
	releaseAuthority,
} from "../configure-standalone-release";

test("release setup persists a private authority and refuses silent rotation or rollback", () => {
	const directory = mkdtempSync(join(tmpdir(), "standalone-authority-"));
	try {
		const key = join(directory, "key");
		const publicKey = releaseAuthority(key, true);
		expect(Buffer.from(publicKey, "base64url").length).toBe(32);
		expect(statSync(key).mode & 0o777).toBe(0o600);
		expect(releaseAuthority(key)).toBe(publicKey);
		expect(() => releaseAuthority(key, true)).toThrow();
		const config = join(directory, "config.json");
		writeFileSync(
			config,
			JSON.stringify({ standalone: { enabled: true }, domain: "api.example" }),
		);
		configureRelease(
			config,
			publicKey,
			"https://cdn.example/standalone/release.jws",
			3,
		);
		const value = JSON.parse(readFileSync(config, "utf8"));
		expect(value.standalone.enabled).toBe(true);
		expect(value.domain).toBe("api.example");
		expect(value.standalone.release_trust.public_keys).toEqual([publicKey]);
		expect(readFileSync(config, "utf8")).not.toContain(
			readFileSync(key, "utf8").trim(),
		);
		expect(() =>
			configureRelease(
				config,
				publicKey,
				"https://cdn.example/standalone/release.jws",
				2,
			),
		).toThrow();
		expect(() =>
			configureRelease(
				config,
				"another-key",
				"https://cdn.example/standalone/release.jws",
				4,
			),
		).toThrow();
		chmodSync(key, 0o644);
		expect(() => releaseAuthority(key)).toThrow();
		chmodSync(key, 0o600);
		symlinkSync(key, join(directory, "link"));
		expect(() => releaseAuthority(join(directory, "link"))).toThrow();
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
});
