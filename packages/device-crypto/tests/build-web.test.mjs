import assert from "node:assert/strict";
import { test } from "node:test";
import { wasmCompilerEnvironment } from "../build-web.mjs";

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
