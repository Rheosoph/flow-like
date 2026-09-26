import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join, resolve } from "node:path";

// ring compiles C for wasm32. Apple's system clang has no WebAssembly backend.
export function wasmCompilerEnvironment({
	env = process.env,
	platform = process.platform,
	probe = (command, args) => {
		try {
			return execFileSync(command, args, {
				encoding: "utf8",
				stdio: ["ignore", "pipe", "ignore"],
				timeout: 5000,
			}).trim();
		} catch {
			return undefined;
		}
	},
	exists = existsSync,
} = {}) {
	if (env["CC_wasm32-unknown-unknown"] || env.CC_wasm32_unknown_unknown)
		return {};
	const suffix = platform === "win32" ? ".exe" : "";
	const candidates = [`clang${suffix}`];
	if (platform === "darwin") {
		candidates.push(
			"/opt/homebrew/opt/llvm/bin/clang",
			"/usr/local/opt/llvm/bin/clang",
		);
		const prefix = probe("brew", ["--prefix", "llvm"]);
		if (prefix) candidates.push(join(prefix, "bin", "clang"));
	}
	const compiler = [...new Set(candidates)].find((candidate) =>
		/^\s*wasm32\s+-/m.test(probe(candidate, ["--print-targets"]) ?? ""),
	);
	if (!compiler) {
		throw new Error(
			"Device cryptography requires clang with a wasm32 backend for ring. " +
				"Install LLVM with WebAssembly support (macOS: brew install llvm), then set " +
				"CC_wasm32_unknown_unknown to its clang executable. Apple system clang cannot build this target.",
		);
	}
	const selected = { CC_wasm32_unknown_unknown: compiler };
	if (!env["AR_wasm32-unknown-unknown"] && !env.AR_wasm32_unknown_unknown) {
		const sibling = join(dirname(compiler), `llvm-ar${suffix}`);
		const archiver = exists(sibling) ? sibling : `llvm-ar${suffix}`;
		if (probe(archiver, ["--version"]))
			selected.AR_wasm32_unknown_unknown = archiver;
	}
	return selected;
}

function build() {
	const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
	const debug = process.argv.includes("--debug");
	const target =
		process.env.CARGO_TARGET_DIR || join(root, "target/device-crypto-web");
	const tools = join(root, "target/device-crypto-tools");
	const lock = readFileSync(join(root, "Cargo.lock"), "utf8");
	const version = lock.match(
		/\[\[package\]\]\s+name = "wasm-bindgen"\s+version = "([^"]+)"/,
	)?.[1];
	if (!version) throw new Error("Cargo.lock does not pin wasm-bindgen");
	const executable =
		process.env.WASM_BINDGEN ||
		join(
			tools,
			"bin",
			process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen",
		);
	const compiler = wasmCompilerEnvironment();
	const run = (command, args) =>
		execFileSync(command, args, {
			cwd: root,
			stdio: "inherit",
			env: { ...process.env, ...compiler, CARGO_TARGET_DIR: target },
		});
	let installed = "";
	try {
		installed = execFileSync(executable, ["--version"], {
			encoding: "utf8",
		}).trim();
	} catch {}
	if (installed !== `wasm-bindgen ${version}`) {
		run("cargo", [
			"install",
			"wasm-bindgen-cli",
			"--version",
			version,
			"--locked",
			"--root",
			tools,
		]);
	}
	run("rustup", ["target", "add", "wasm32-unknown-unknown"]);
	run("cargo", [
		"build",
		"-p",
		"flow-like-device-crypto",
		"--features",
		"wasm",
		"--target",
		"wasm32-unknown-unknown",
		...(debug ? [] : ["--release"]),
	]);
	const output = join(root, "apps/web/public/device-crypto");
	mkdirSync(output, { recursive: true });
	run(executable, [
		join(
			target,
			"wasm32-unknown-unknown",
			debug ? "debug" : "release",
			"flow_like_device_crypto.wasm",
		),
		"--target",
		"web",
		"--out-dir",
		output,
		"--out-name",
		"flow_like_device_crypto",
	]);
	cpSync(output, join(root, "apps/desktop/public/device-crypto"), {
		recursive: true,
	});
}

if (
	process.argv[1] &&
	import.meta.url === pathToFileURL(resolve(process.argv[1])).href
)
	build();
