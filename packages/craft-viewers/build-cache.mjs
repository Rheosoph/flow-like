import {
	mkdirSync,
	readFileSync,
	statSync,
	utimesSync,
	writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { acquireBuildLock } from "./build-lock.mjs";

export function acquireBuildLockWithInputs({
	directory,
	label,
	readFingerprint,
}) {
	// Imports are already loaded. A lock wait must not assign newer inputs to them.
	const fingerprint = readFingerprint();
	const releaseLock = acquireBuildLock({ directory, label });
	try {
		if (readFingerprint() !== fingerprint)
			throw new Error("The build input fingerprint changed.");
	} catch (error) {
		releaseLock();
		throw new Error(
			`${label} build inputs changed while waiting for its lock. Retry the build to load the updated inputs.`,
			{ cause: error },
		);
	}
	return { fingerprint, releaseLock };
}

export function buildCacheConfiguration({ root, id, env = process.env }) {
	const requested = env.CRAFT_BUILD_CACHE ?? "shared";
	if (!["auto", "shared"].includes(requested))
		throw new Error("CRAFT_BUILD_CACHE must be auto or shared.");
	const directory =
		id === "wordcraft"
			? join(root, "target/wordcraft")
			: join(root, "target/craft-viewers", id);
	const override =
		id === "wordcraft"
			? env.WORDCRAFT_TARGET_DIR
			: env[`CRAFT_TARGET_${id.toUpperCase()}`];
	const shared =
		requested === "shared" ||
		(id !== "wordcraft" && env.CRAFT_BUILD_CONCURRENCY === "1");
	const cargoTarget = override
		? override
		: shared
			? join(root, "target/craft-viewers/cargo")
			: join(directory, "cargo");
	return {
		mode: requested,
		cargoTarget,
		// Keep the legacy mode's compiler inputs intact for timing comparisons.
		linkerDirectory:
			requested === "shared"
				? join(root, "target/craft-viewers/linker")
				: join(directory, "linker"),
		contentAddressed: requested === "shared",
	};
}

// Applying an unchanged adapter must not invalidate a warm Cargo build.
export function writeSourceFile(source, path, contents) {
	const destination = join(source, path);
	const bytes = Buffer.isBuffer(contents) ? contents : Buffer.from(contents);
	try {
		if (readFileSync(destination).equals(bytes)) return;
	} catch (error) {
		if (error.code !== "ENOENT") throw error;
	}
	mkdirSync(dirname(destination), { recursive: true });
	writeFileSync(destination, bytes);
}

export function createSourceWriter(source) {
	const previous = new Map();
	return {
		write(path, contents) {
			if (!previous.has(path)) {
				try {
					const destination = join(source, path);
					previous.set(path, {
						stat: statSync(destination),
						contents: readFileSync(destination),
					});
				} catch (error) {
					if (error.code !== "ENOENT") throw error;
					previous.set(path, null);
				}
			}
			writeSourceFile(source, path, contents);
		},
		finish() {
			// Some adapters read intermediate files, then append their MCP bridge.
			// Restore timestamps only when the complete preparation kept the same bytes.
			for (const [path, original] of previous) {
				if (!original) continue;
				const destination = join(source, path);
				if (
					readFileSync(destination).equals(original.contents) &&
					statSync(destination).mtimeMs !== original.stat.mtimeMs
				)
					utimesSync(
						destination,
						original.stat.atimeMs / 1000,
						original.stat.mtimeMs / 1000,
					);
			}
			previous.clear();
		},
	};
}

export function buildSwitch(env, name) {
	const value = env[name];
	if (value !== undefined && value !== "0" && value !== "1")
		throw new Error(`${name} must be 0 or 1.`);
	return value === "1";
}
