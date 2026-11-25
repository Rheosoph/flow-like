import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
	existsSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	statSync,
	utimesSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import {
	buildCacheConfiguration,
	buildSwitch,
	createSourceWriter,
	writeSourceFile,
} from "./build-cache.mjs";
import { acquireBuildLock } from "./build-lock.mjs";

for (const change of ["changed", "removed", "unchanged"]) {
	test(
		`lock wait ${change === "unchanged" ? "accepts unchanged inputs" : `rejects ${change} inputs before stamping stale imports`}`,
		{ timeout: 10000 },
		async (t) => {
			const directory = mkdtempSync(join(tmpdir(), "craft-input-lock-"));
			t.after(() => rmSync(directory, { recursive: true, force: true }));
			const input = join(directory, "input.mjs");
			const stamp = join(directory, "runtime.json");
			writeFileSync(input, 'export const version = "loaded";\n');
			const release = acquireBuildLock({ directory, label: "fixture" });
			t.after(release);
			const child = spawn(
				process.execPath,
				[
					"--input-type=module",
					"-e",
					`
			import { createHash } from "node:crypto";
			import { readFileSync, writeFileSync } from "node:fs";
				import { version } from ${JSON.stringify(pathToFileURL(input).href)};
			import { acquireBuildLockWithInputs } from ${JSON.stringify(new URL("./build-cache.mjs", import.meta.url).href)};
			const { fingerprint, releaseLock } = acquireBuildLockWithInputs({
				directory: ${JSON.stringify(directory)},
				label: "fixture",
				readFingerprint: () => createHash("sha256").update(readFileSync(${JSON.stringify(input)})).digest("hex"),
			});
			writeFileSync(${JSON.stringify(stamp)}, JSON.stringify({ version, fingerprint }));
			releaseLock();
		`,
				],
				{ stdio: ["ignore", "pipe", "pipe"] },
			);
			t.after(() => child.kill());
			let stderr = "";
			child.stderr.on("data", (data) => {
				stderr += data;
			});
			const completed = new Promise((resolve, reject) => {
				child.once("error", reject);
				child.once("close", resolve);
			});
			await new Promise((resolve, reject) => {
				let stdout = "";
				child.stdout.on("data", (data) => {
					stdout += data;
					if (stdout.includes("Waiting for the active fixture build"))
						resolve();
				});
				child.once("error", reject);
				child.once("exit", (code) =>
					reject(
						new Error(`Builder exited before waiting (${code}): ${stderr}`),
					),
				);
			});
			if (change === "changed")
				writeFileSync(input, 'export const version = "updated";\n');
			else if (change === "removed") rmSync(input);
			release();
			const code = await completed;
			assert.equal(existsSync(join(directory, "build.lock")), false);
			if (change === "unchanged") {
				assert.equal(code, 0, stderr);
				const result = JSON.parse(readFileSync(stamp, "utf8"));
				assert.equal(result.version, "loaded");
				assert.match(result.fingerprint, /^[a-f0-9]{64}$/);
			} else {
				assert.notEqual(code, 0);
				assert.match(
					stderr,
					/build inputs changed while waiting.*Retry the build/,
				);
				assert.equal(existsSync(stamp), false);
			}
		},
	);
}

test("default mode shares Cargo and linker locations across WordCraft and viewers", () => {
	const root = join(tmpdir(), "craft-root");
	const configurations = ["wordcraft", "gridcraft", "pdfcraft"].map((id) =>
		buildCacheConfiguration({
			root,
			id,
			env: { CRAFT_BUILD_CONCURRENCY: "4" },
		}),
	);
	for (const configuration of configurations) {
		assert.deepEqual(configuration, configurations[0]);
		assert.equal(
			configuration.cargoTarget,
			join(root, "target/craft-viewers/cargo"),
		);
		assert.equal(configuration.contentAddressed, true);
		assert.equal(configuration.mode, "shared");
	}
});

test("auto mode retains existing parallel and sequential cache locations", () => {
	const root = join(tmpdir(), "craft-root");
	for (const concurrency of [undefined, "1", "4"]) {
		const env = {
			CRAFT_BUILD_CACHE: "auto",
			CRAFT_BUILD_CONCURRENCY: concurrency,
		};
		const word = buildCacheConfiguration({ root, id: "wordcraft", env });
		const grid = buildCacheConfiguration({ root, id: "gridcraft", env });
		assert.equal(word.cargoTarget, join(root, "target/wordcraft/cargo"));
		assert.equal(
			grid.cargoTarget,
			join(
				root,
				concurrency === "1"
					? "target/craft-viewers/cargo"
					: "target/craft-viewers/gridcraft/cargo",
			),
		);
		assert.equal(
			grid.linkerDirectory,
			join(root, "target/craft-viewers/gridcraft/linker"),
		);
		assert.equal(grid.contentAddressed, false);
	}
});

test("explicit cache overrides are preserved and invalid modes fail", () => {
	const root = join(tmpdir(), "craft-root");
	for (const [id, key] of [
		["wordcraft", "WORDCRAFT_TARGET_DIR"],
		["gridcraft", "CRAFT_TARGET_GRIDCRAFT"],
	]) {
		assert.equal(
			buildCacheConfiguration({
				root,
				id,
				env: { CRAFT_BUILD_CACHE: "shared", [key]: "custom" },
			}).cargoTarget,
			"custom",
		);
	}
	assert.throws(
		() =>
			buildCacheConfiguration({
				root,
				id: "wordcraft",
				env: { CRAFT_BUILD_CACHE: "typo" },
			}),
		/auto or shared/,
	);
	assert.equal(buildSwitch({}, "CRAFT_REBUILD"), false);
	assert.equal(buildSwitch({ CRAFT_REBUILD: "1" }, "CRAFT_REBUILD"), true);
	assert.throws(
		() => buildSwitch({ CRAFT_REBUILD: "yes" }, "CRAFT_REBUILD"),
		/must be 0 or 1/,
	);
});

test("unchanged source writes preserve timestamps while edits replace contents", (t) => {
	const source = mkdtempSync(join(tmpdir(), "craft-source-"));
	t.after(() => rmSync(source, { recursive: true, force: true }));
	const path = "crates/ui/src/lib.rs";
	writeSourceFile(source, path, "first\n");
	const destination = join(source, path);
	utimesSync(destination, 100, 100);
	writeSourceFile(source, path, "first\n");
	assert.equal(statSync(destination).mtimeMs, 100000);
	writeSourceFile(source, path, "second\n");
	assert.equal(readFileSync(destination, "utf8"), "second\n");
	assert.notEqual(statSync(destination).mtimeMs, 100000);
});

test("adapter passes restore timestamps only when their final contents are unchanged", (t) => {
	const source = mkdtempSync(join(tmpdir(), "craft-prepare-"));
	t.after(() => rmSync(source, { recursive: true, force: true }));
	writeSourceFile(source, "lib.rs", "upstream\nbridge\n");
	writeSourceFile(source, "changed.rs", "old\n");
	for (const path of ["lib.rs", "changed.rs"])
		utimesSync(join(source, path), 100, 100);
	const writer = createSourceWriter(source);
	writer.write("lib.rs", "upstream\n");
	assert.equal(readFileSync(join(source, "lib.rs"), "utf8"), "upstream\n");
	writer.write("lib.rs", "upstream\nbridge\n");
	writer.write("changed.rs", "new\n");
	writer.write("new.rs", "added\n");
	writer.finish();
	assert.equal(statSync(join(source, "lib.rs")).mtimeMs, 100000);
	assert.notEqual(statSync(join(source, "changed.rs")).mtimeMs, 100000);
	assert.equal(readFileSync(join(source, "new.rs"), "utf8"), "added\n");
	writer.finish();
});

test("binary dependency writes compare bytes without conflating invalid UTF-8", (t) => {
	const source = mkdtempSync(join(tmpdir(), "craft-binary-source-"));
	t.after(() => rmSync(source, { recursive: true, force: true }));
	writeSourceFile(source, "data.bin", Buffer.from([0x80]));
	utimesSync(join(source, "data.bin"), 100, 100);
	writeSourceFile(source, "data.bin", Buffer.from([0x80]));
	assert.equal(statSync(join(source, "data.bin")).mtimeMs, 100000);
	const writer = createSourceWriter(source);
	writer.write("data.bin", Buffer.from([0x81]));
	writer.finish();
	assert.deepEqual(readFileSync(join(source, "data.bin")), Buffer.from([0x81]));
	assert.notEqual(statSync(join(source, "data.bin")).mtimeMs, 100000);
});
