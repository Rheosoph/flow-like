import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const DESKTOP_DIR = path.resolve(SCRIPT_DIR, "..");
const SRC_TAURI_DIR = path.join(DESKTOP_DIR, "src-tauri");
const PACKAGE_DIR = path.join(SRC_TAURI_DIR, "apple", "FlowLikeMLX");
const DERIVED_DATA_DIR = path.join(SRC_TAURI_DIR, "target", "mlx-swift");
const OUTPUT_DIR = path.join(SRC_TAURI_DIR, "binaries", "mac", "arm");
const RESOURCE_DIR = path.join(OUTPUT_DIR, "mlx-resources");
const TARGET_TRIPLE = "aarch64-apple-darwin";

type Configuration = "Debug" | "Release";

function configurationFromArgs(): Configuration {
	let configuration: Configuration = "Debug";
	for (let index = 2; index < process.argv.length; index++) {
		const argument = process.argv[index];
		if (argument === "--configuration") {
			const value = process.argv[++index]?.toLowerCase();
			if (value === "debug") configuration = "Debug";
			else if (value === "release") configuration = "Release";
			else throw new Error("--configuration must be Debug or Release");
			continue;
		}
		if (argument === "--help" || argument === "-h") {
			console.log(
				"Usage: bun scripts/prepare-mlx.ts [--configuration Debug|Release]",
			);
			process.exit(0);
		}
		throw new Error(`Unknown option: ${argument}`);
	}
	return configuration;
}

function findNamedFile(root: string, name: string): string | undefined {
	if (!fs.existsSync(root)) return undefined;
	const direct = path.join(root, name);
	if (fs.existsSync(direct) && fs.statSync(direct).isFile()) return direct;
	for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
		const candidate = path.join(root, entry.name);
		if (entry.isFile() && entry.name === name) return candidate;
		// Debug symbol bundles contain a same-named DWARF companion that must
		// never be mistaken for the executable.
		if (entry.isDirectory() && !entry.name.endsWith(".dSYM")) {
			const nested = findNamedFile(candidate, name);
			if (nested) return nested;
		}
	}
	return undefined;
}

function collectResourceBundles(root: string): string[] {
	if (!fs.existsSync(root)) return [];
	const bundles: string[] = [];
	for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
		const candidate = path.join(root, entry.name);
		if (!entry.isDirectory()) continue;
		if (entry.name.endsWith(".bundle")) {
			bundles.push(candidate);
			continue;
		}
		// Test and debug products can embed copies of the same SwiftPM resource
		// bundles that also exist at the products root. They are not runtime
		// resources for the helper and would otherwise cause duplicate names.
		if (entry.name.endsWith(".xctest") || entry.name.endsWith(".dSYM")) {
			continue;
		}
		bundles.push(...collectResourceBundles(candidate));
	}
	return bundles;
}

function main(): void {
	const configuration = configurationFromArgs();
	if (process.platform !== "darwin") {
		throw new Error(
			"The MLX Swift helper can only be built on macOS with Xcode",
		);
	}
	if (!fs.existsSync(path.join(PACKAGE_DIR, "Package.swift"))) {
		throw new Error(`MLX Swift package is missing at ${PACKAGE_DIR}`);
	}

	fs.mkdirSync(DERIVED_DATA_DIR, { recursive: true });
	fs.mkdirSync(OUTPUT_DIR, { recursive: true });

	console.log(`Preparing the MLX Swift helper (${configuration})...`);
	const build = spawnSync(
		"xcodebuild",
		[
			"-quiet",
			"-scheme",
			"flow-like-mlx",
			"-configuration",
			configuration,
			"-destination",
			"generic/platform=macOS",
			"-derivedDataPath",
			DERIVED_DATA_DIR,
			"ARCHS=arm64",
			"ONLY_ACTIVE_ARCH=YES",
			"MACOSX_DEPLOYMENT_TARGET=14.0",
			"CODE_SIGNING_ALLOWED=NO",
			"build",
		],
		{
			cwd: PACKAGE_DIR,
			stdio: "inherit",
		},
	);
	if (build.error) throw build.error;
	if (build.status !== 0) {
		throw new Error(
			`xcodebuild failed with status ${build.status ?? "unknown"}`,
		);
	}

	const products = path.join(
		DERIVED_DATA_DIR,
		"Build",
		"Products",
		configuration,
	);
	const executable = findNamedFile(products, "flow-like-mlx");
	if (!executable) {
		throw new Error(
			`xcodebuild did not produce flow-like-mlx under ${products}`,
		);
	}

	const stagedExecutable = path.join(
		OUTPUT_DIR,
		`flow-like-mlx-service-${TARGET_TRIPLE}`,
	);
	fs.copyFileSync(executable, stagedExecutable);
	fs.chmodSync(stagedExecutable, 0o755);

	fs.rmSync(RESOURCE_DIR, { recursive: true, force: true });
	fs.mkdirSync(RESOURCE_DIR, { recursive: true });
	const bundles = collectResourceBundles(products);
	if (bundles.length === 0) {
		throw new Error(
			`No Swift resource bundles were produced under ${products}; MLX Metal shaders would be missing`,
		);
	}
	const bundleNames = new Set<string>();
	for (const bundle of bundles) {
		const bundleName = path.basename(bundle);
		if (bundleNames.has(bundleName)) {
			throw new Error(`Duplicate Swift resource bundle name: ${bundleName}`);
		}
		bundleNames.add(bundleName);
		fs.cpSync(bundle, path.join(RESOURCE_DIR, bundleName), {
			recursive: true,
		});
	}

	console.log(
		`Staged the MLX helper and ${bundles.length} Swift resource bundle(s) for ${TARGET_TRIPLE}.`,
	);
}

main();
