import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	rmSync,
	statSync,
	utimesSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { Worker } from "node:worker_threads";
import {
	rustLibraryEnvironment,
	wasmCompilerEnvironment,
	wasmLinkerEnvironment,
} from "../build-web.mjs";

test("macOS exposes the active Rust LLVM library and keeps default fallback paths", () => {
	assert.deepEqual(
		rustLibraryEnvironment({
			env: { HOME: "/Users/developer" },
			platform: "darwin",
			probe: () => "/toolchains/current",
			exists: (path) => path === "/toolchains/current/lib/libLLVM.dylib",
		}),
		{
			DYLD_FALLBACK_LIBRARY_PATH:
				"/toolchains/current/lib:/Users/developer/lib:/usr/local/lib:/usr/lib",
		},
	);
});

test("Rust library discovery preserves configured paths without duplicating or mutating them", () => {
	const env = Object.freeze({
		DYLD_FALLBACK_LIBRARY_PATH:
			"/custom/lib:/toolchains/current/lib:/other/lib:/custom/lib",
	});
	assert.deepEqual(
		rustLibraryEnvironment({
			env,
			platform: "darwin",
			probe: () => "/toolchains/current",
			exists: (path) => path === "/toolchains/current/lib/libLLVM.dylib",
		}),
		{
			DYLD_FALLBACK_LIBRARY_PATH:
				"/toolchains/current/lib:/custom/lib:/other/lib",
		},
	);
	assert.equal(
		env.DYLD_FALLBACK_LIBRARY_PATH,
		"/custom/lib:/toolchains/current/lib:/other/lib:/custom/lib",
	);
});

test("Rust library discovery uses the configured Rust compiler", () => {
	assert.deepEqual(
		rustLibraryEnvironment({
			env: {
				RUSTC: "/custom/bin/rustc",
				DYLD_FALLBACK_LIBRARY_PATH: "/custom/lib",
			},
			platform: "darwin",
			probe: (command) => {
				if (command === "/custom/bin/rustc") return "/custom/toolchain";
				throw new Error("The configured Rust compiler must select its sysroot");
			},
			exists: (path) => path === "/custom/toolchain/lib/libLLVM.dylib",
		}),
		{
			DYLD_FALLBACK_LIBRARY_PATH: "/custom/toolchain/lib:/custom/lib",
		},
	);
});

test("Linux and Windows do not probe for macOS Rust libraries", () => {
	for (const platform of ["linux", "win32"]) {
		assert.deepEqual(
			rustLibraryEnvironment({
				env: {},
				platform,
				probe: () => {
					throw new Error("Only macOS needs a library search path");
				},
			}),
			{},
		);
	}
});

test("Rust toolchains without a shared LLVM library keep their existing environment", () => {
	assert.deepEqual(
		rustLibraryEnvironment({
			env: { DYLD_FALLBACK_LIBRARY_PATH: "/custom/lib" },
			platform: "darwin",
			probe: () => "/toolchains/current",
			exists: () => false,
		}),
		{},
	);
});

test("an explicit WebAssembly linker is preserved without probing Rust", () => {
	assert.deepEqual(
		wasmLinkerEnvironment({
			env: {
				CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER: "/custom/bin/wasm-ld",
			},
			platform: "darwin",
			probe: () => {
				throw new Error("An explicit linker must not be replaced");
			},
		}),
		{},
	);
});

test("linker discovery creates no wrapper when the runtime workaround is unnecessary", (t) => {
	const temporary = mkdtempSync(join(tmpdir(), "device-crypto-linker-"));
	t.after(() => rmSync(temporary, { recursive: true, force: true }));
	const directory = join(temporary, "tools");
	for (const platform of ["linux", "win32", "darwin"]) {
		assert.deepEqual(
			wasmLinkerEnvironment({
				directory,
				env: {},
				platform,
				probe: () => "/toolchains/current",
				exists: () => false,
			}),
			{},
		);
	}
	assert.equal(existsSync(directory), false);
});

test(
	"the linker launcher restores the runtime path and forwards literal arguments through a shell",
	{ skip: process.platform === "win32" },
	(t) => {
		const temporary = mkdtempSync(join(tmpdir(), "device-crypto-linker-"));
		t.after(() => rmSync(temporary, { recursive: true, force: true }));
		const sysroot = join(temporary, "toolchain ' $VALUE $(printf substituted)");
		const targetLib = join(sysroot, "lib", "rustlib", "host", "lib");
		const linker = join(dirname(targetLib), "bin", "rust-lld");
		mkdirSync(dirname(linker), { recursive: true });
		writeFileSync(join(sysroot, "lib", "libLLVM.dylib"), "");
		writeFileSync(
			linker,
			`#!${process.execPath}\nprocess.stdout.write(JSON.stringify({ args: process.argv.slice(2), fallback: process.env.DYLD_FALLBACK_LIBRARY_PATH }));\n`,
			{ mode: 0o755 },
		);
		const fallback = "/custom ' libraries/$VALUE:/other/lib";
		const selected = wasmLinkerEnvironment({
			directory: join(temporary, "generated tools"),
			env: { DYLD_FALLBACK_LIBRARY_PATH: fallback },
			platform: "darwin",
			probe: (_command, args) => (args[1] === "sysroot" ? sysroot : targetLib),
		});
		assert.equal(
			selected.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER,
			join(temporary, "generated tools", "rust-lld"),
		);
		const args = [
			"-flavor",
			"wasm",
			"-o",
			"output ' $VALUE.wasm",
			"$(printf substituted)",
			"",
		];
		const result = JSON.parse(
			execFileSync(selected.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER, args, {
				encoding: "utf8",
				env: {},
			}),
		);
		assert.deepEqual(result, {
			args,
			fallback: `${join(sysroot, "lib")}:${fallback}`,
		});
	},
);

test("shared linker launchers keep their file and separate changes to paths and libraries", (t) => {
	const directory = mkdtempSync(join(tmpdir(), "device-crypto-shared-linker-"));
	t.after(() => rmSync(directory, { recursive: true, force: true }));
	const select = (target = "host", fallback = "/custom/lib") =>
		wasmLinkerEnvironment({
			directory,
			contentAddressed: true,
			env: { DYLD_FALLBACK_LIBRARY_PATH: fallback },
			platform: "darwin",
			exists: () => true,
			probe: (_command, args) =>
				args[1] === "sysroot"
					? "/toolchains/current"
					: `/toolchains/current/lib/rustlib/${target}/lib`,
		}).CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER;
	const first = select();
	const contents = readFileSync(first, "utf8");
	utimesSync(first, 1, 1);
	const original = statSync(first);
	assert.equal(select(), first);
	assert.equal(statSync(first).mtimeMs, original.mtimeMs);
	assert.equal(statSync(first).ino, original.ino);
	const otherLinker = select("other-host");
	const otherLibraries = select("host", "/other/lib");
	assert.equal(new Set([first, otherLinker, otherLibraries]).size, 3);
	assert.equal(readFileSync(first, "utf8"), contents);
	assert.notEqual(readFileSync(otherLinker, "utf8"), contents);
	assert.notEqual(readFileSync(otherLibraries, "utf8"), contents);
	assert.deepEqual(readdirSync(dirname(first)), ["rust-lld"]);
	writeFileSync(first, "changed");
	assert.throws(select, /shared WebAssembly linker launcher was modified/);
	assert.equal(readFileSync(first, "utf8"), "changed");
});

test(
	"parallel builders publish one complete shared linker launcher",
	{ timeout: 10000 },
	async (t) => {
		const directory = mkdtempSync(
			join(tmpdir(), "device-crypto-shared-linker-"),
		);
		t.after(() => rmSync(directory, { recursive: true, force: true }));
		const gate = new SharedArrayBuffer(4);
		const workers = [];
		const ready = [];
		const finished = [];
		for (let index = 0; index < 6; index++) {
			const worker = new Worker(
				`
			const { parentPort, workerData } = require("node:worker_threads");
			const assert = require("node:assert/strict");
			const { readFileSync } = require("node:fs");
			const { dirname, join } = require("node:path");
			import(workerData.module).then(({ wasmLinkerEnvironment }) => {
				parentPort.postMessage("ready");
				Atomics.wait(new Int32Array(workerData.gate), 0, 0);
				const sysroot = join(workerData.directory, "toolchain");
				const targetLib = join(sysroot, "lib/rustlib/host/lib");
				const expected = "#!/bin/sh\\nexport DYLD_FALLBACK_LIBRARY_PATH='" + join(sysroot, "lib") + ":/custom/lib'\\nexec '" + join(dirname(targetLib), "bin/rust-lld") + "' \\"$@\\"\\n";
				let path;
				for (let attempt = 0; attempt < 20; attempt++) {
					path = wasmLinkerEnvironment({
						directory: workerData.directory,
						contentAddressed: true,
						env: { DYLD_FALLBACK_LIBRARY_PATH: "/custom/lib" },
						platform: "darwin",
						exists: () => true,
						probe: (_command, args) => args[1] === "sysroot" ? sysroot : targetLib,
					}).CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER;
					assert.equal(readFileSync(path, "utf8"), expected);
				}
				parentPort.postMessage({ path });
			});
			`,
				{
					eval: true,
					workerData: {
						directory,
						gate,
						module: new URL("../build-web.mjs", import.meta.url).href,
					},
				},
			);
			workers.push(worker);
			ready.push(
				new Promise((resolve, reject) => {
					worker.once("error", reject);
					worker.on("message", (message) => message === "ready" && resolve());
				}),
			);
			finished.push(
				new Promise((resolve, reject) => {
					worker.once("error", reject);
					worker.on(
						"message",
						(message) => message.path && resolve(message.path),
					);
				}),
			);
		}
		t.after(() => Promise.all(workers.map((worker) => worker.terminate())));
		await Promise.all(ready);
		Atomics.store(new Int32Array(gate), 0, 1);
		Atomics.notify(new Int32Array(gate), 0);
		const paths = await Promise.all(finished);
		assert.equal(new Set(paths).size, 1);
		assert.deepEqual(readdirSync(dirname(paths[0])), ["rust-lld"]);
	},
);

test("target-specific compiler overrides are preserved without probing", () => {
	for (const name of [
		"CC_wasm32_unknown_unknown",
		"CC_wasm32-unknown-unknown",
	]) {
		assert.deepEqual(
			wasmCompilerEnvironment({
				env: { [name]: "custom compiler wrapper" },
				probe: () => {
					throw new Error("Explicit compiler must not be overridden");
				},
			}),
			{},
		);
	}
});

test("macOS selects Homebrew LLVM when Apple clang cannot target wasm32", () => {
	const result = wasmCompilerEnvironment({
		env: {},
		platform: "darwin",
		exists: (path) => path === "/opt/homebrew/opt/llvm/bin/llvm-ar",
		probe: (command, args) => {
			if (command === "/opt/homebrew/opt/llvm/bin/clang")
				return "  wasm32 - WebAssembly 32-bit\n";
			if (command.endsWith("llvm-ar") && args[0] === "--version")
				return "LLVM version 21";
			return undefined;
		},
	});
	assert.deepEqual(result, {
		CC_wasm32_unknown_unknown: "/opt/homebrew/opt/llvm/bin/clang",
		AR_wasm32_unknown_unknown: "/opt/homebrew/opt/llvm/bin/llvm-ar",
	});
});

test("PATH LLVM works on Linux and preserves a configured target archiver", () => {
	assert.deepEqual(
		wasmCompilerEnvironment({
			env: { AR_wasm32_unknown_unknown: "custom-ar" },
			platform: "linux",
			exists: () => false,
			probe: (command) =>
				command === "clang" ? "  wasm32 - WebAssembly 32-bit" : undefined,
		}),
		{ CC_wasm32_unknown_unknown: "clang" },
	);
});

test("missing wasm32 compiler fails with a configuration instruction", () => {
	assert.throws(
		() =>
			wasmCompilerEnvironment({
				env: {},
				platform: "linux",
				probe: () => undefined,
			}),
		/CC_wasm32_unknown_unknown/,
	);
});
