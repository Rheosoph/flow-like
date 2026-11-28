import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	cpSync,
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	rmSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";
import { wasmLinkerEnvironment } from "../device-crypto/build-web.mjs";
import {
	acquireBuildLockWithInputs,
	buildCacheConfiguration,
	buildSwitch,
	createSourceWriter,
	writeSourceFile,
} from "./build-cache.mjs";
import { createBuildTimings } from "./build-timings.mjs";
import { ensureWasmBindgen } from "./build-tools.mjs";
import {
	assertPinnedLockPackages,
	assertPlatformMediaDependencies,
	usesPlatformMedia,
} from "./codec-policy.mjs";
import { prepareInventory } from "./inventory-patch.mjs";
import {
	assertNoBrandArtwork,
	bundleLicenses,
	hashLicenseInputs,
} from "./licenses.mjs";
import { initializeWasmStartup, prepareWasmStartup } from "./wasm-startup.mjs";

const suiteDirectory = dirname(fileURLToPath(import.meta.url));
const root = resolve(suiteDirectory, "../..");
const id = process.argv[2];
if (!id || !/^[a-z]+craft$/.test(id))
	throw new Error("Pass a Craft runtime name.");
const timings = createBuildTimings({ id });
timings.phase("prepare");
const buildCache = buildCacheConfiguration({ root, id });
const rebuild = buildSwitch(process.env, "CRAFT_REBUILD");
const skipStage = buildSwitch(process.env, "CRAFT_SKIP_STAGE");
let cacheHit = false;
let succeeded = false;
process.once("exit", () => {
	try {
		timings.finish({
			cached: cacheHit,
			success: succeeded,
			cacheMode: buildCache.mode,
		});
	} catch (error) {
		console.error(`Could not write ${id} build timings: ${error.message}`);
		process.exitCode = 1;
	}
});
const directory = join(suiteDirectory, id);
const toolchain =
	process.env.RUSTUP_TOOLCHAIN ||
	readFileSync(join(root, "rust-toolchain.toml"), "utf8").match(
		/^channel\s*=\s*"([^"]+)"/m,
	)?.[1];
if (!toolchain)
	throw new Error("The repository Rust toolchain is not configured.");
const buildEnvironment = { ...process.env, RUSTUP_TOOLCHAIN: toolchain };
let pin = JSON.parse(readFileSync(join(directory, "upstream.json"), "utf8"));
if (!/^[a-f0-9]{40}$/.test(pin.revision))
	throw new Error("Craft runtimes must be pinned to an exact Git commit.");
const cache = join(root, "target/craft-viewers", id);
const { fingerprint, releaseLock } = acquireBuildLockWithInputs({
	directory: cache,
	label: pin.label,
	readFingerprint: inputFingerprint,
});
// Keep the pin used by this build inside the editor lock.
pin = JSON.parse(readFileSync(join(directory, "upstream.json"), "utf8"));
if (!/^[a-f0-9]{40}$/.test(pin.revision))
	throw new Error("Craft runtimes must be pinned to an exact Git commit.");
const source =
	process.env[`CRAFT_SOURCE_${id.toUpperCase()}`] ||
	join(cache, "source", pin.revision);
const output = join(cache, "runtime");
function inputFingerprint() {
	const hash = createHash("sha256").update(toolchain);
	for (const base of [directory, suiteDirectory]) {
		for (const name of readdirSync(base).sort()) {
			const path = join(base, name);
			if (!statSync(path).isFile()) continue;
			hash.update(path.slice(root.length)).update(readFileSync(path));
		}
	}
	for (const name of [
		"packages/device-crypto/build-web.mjs",
		"rust-toolchain.toml",
	])
		hash.update(name).update(readFileSync(join(root, name)));
	hashLicenseInputs(hash);
	return hash.digest("hex");
}
const stampPath = join(output, "runtime.json");
const targets = [
	`apps/web/public/craft/${id}`,
	`apps/desktop/public/craft/${id}`,
];

function run(command, args, options = {}) {
	return execFileSync(command, args, {
		cwd: source,
		stdio: "inherit",
		env: buildEnvironment,
		...options,
	});
}

function original(path) {
	return run("git", ["show", `${pin.revision}:${path}`], {
		encoding: "utf8",
		stdio: ["ignore", "pipe", "inherit"],
		maxBuffer: 32 * 1024 * 1024,
	});
}

function replaceOnce(text, search, replacement, name) {
	if (text.split(search).length !== 2) {
		throw new Error(
			`${pin.label} ${pin.revision} changed ${name}; review the host adapter before upgrading.`,
		);
	}
	return text.replace(search, replacement);
}

function digest(path) {
	return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function cached() {
	if (!existsSync(stampPath)) return false;
	try {
		const stamp = JSON.parse(readFileSync(stampPath, "utf8"));
		return (
			stamp.fingerprint === fingerprint &&
			Object.entries(stamp.files).every(
				([name, sha]) =>
					existsSync(join(output, name)) && digest(join(output, name)) === sha,
			)
		);
	} catch {
		return false;
	}
}

function stage() {
	for (const target of targets) {
		rmSync(join(root, target), { recursive: true, force: true });
		mkdirSync(join(root, target), { recursive: true });
		cpSync(output, join(root, target), { recursive: true });
	}
	console.log(
		`${pin.label} ${pin.version} (${pin.revision.slice(0, 12)}) ready for web and desktop.`,
	);
}

cacheHit = !rebuild && cached();
if (!cacheHit) {
	mkdirSync(source, { recursive: true });
	if (!existsSync(join(source, ".git"))) {
		run("git", ["init", "--quiet"]);
	}
	let revision = "";
	try {
		revision = run("git", ["rev-parse", "HEAD"], {
			encoding: "utf8",
			stdio: "pipe",
		}).trim();
	} catch {}
	if (
		revision !== pin.revision &&
		process.env[`CRAFT_SOURCE_${id.toUpperCase()}`] &&
		revision
	) {
		throw new Error(
			`Craft source override must be checked out at ${pin.revision}.`,
		);
	}
	if (revision !== pin.revision) {
		run("git", ["fetch", "--depth", "1", pin.repository, pin.revision]);
		run("git", ["checkout", "--detach", pin.revision]);
	}
	const upstreamVersion = original("Cargo.toml").match(
		/\[workspace.package\]\s+version = "([^"]+)"/,
	)?.[1];
	if (upstreamVersion !== pin.version)
		throw new Error(`Set upstream.json version to ${upstreamVersion}.`);

	const lock = original("Cargo.lock");
	const lockedBindgen = lock.match(
		/\[\[package\]\]\nname = "wasm-bindgen"\nversion = "([^"]+)"/,
	)?.[1];
	if (lockedBindgen !== pin.wasmBindgen)
		throw new Error(`Set ${id}/upstream.json wasmBindgen to ${lockedBindgen}.`);
	const { default: prepare } = await import(
		pathToFileURL(join(directory, "patch.mjs")).href
	);
	const sourceWriter = createSourceWriter(source);
	const { write } = sourceWriter;
	const prepared =
		(await prepare({
			source,
			directory,
			pin,
			original,
			write,
			replaceOnce,
			run,
			root,
		})) || {};
	prepareWasmStartup({ id, source, write, original });
	const inventoryOverrides = prepareInventory({
		id,
		root,
		source,
		original,
		write,
		replaceOnce,
	});
	if (inventoryOverrides.length) {
		prepared.reconcileLock = true;
		prepared.pinnedSourceOverrides = [
			...(prepared.pinnedSourceOverrides || []),
			...inventoryOverrides,
		];
	}

	const executable = ensureWasmBindgen({
		root,
		version: pin.wasmBindgen,
		env: buildEnvironment,
		run,
	});
	const { cargoTarget } = buildCache;
	const env = {
		...buildEnvironment,
		...wasmLinkerEnvironment({
			directory: buildCache.linkerDirectory,
			contentAddressed: buildCache.contentAddressed,
			env: buildEnvironment,
		}),
		CARGO_TARGET_DIR: cargoTarget,
		CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS || "4",
		// Release jobs pass native linker shims through these. They cannot be linked
		// into WebAssembly; the explicit --target below also overrides CARGO_BUILD_TARGET.
		RUSTFLAGS: undefined,
		CARGO_ENCODED_RUSTFLAGS: undefined,
	};
	// Optional system font checkouts would change the runtime without changing its
	// pin or cache key. This build uses only upstream's bundled, attributed fonts.
	env.CRAFT_FONTS_DIR = join(
		root,
		"target/craft-viewers/bundled-fonts-only-v1",
	);
	mkdirSync(env.CRAFT_FONTS_DIR, { recursive: true });
	// Upstream watches these optional manifests even when absent. Empty files keep
	// the bundled font selection unchanged and give Cargo stable shared inputs.
	for (const manifest of ["manifest.txt", "latin-manifest.txt"])
		writeSourceFile(env.CRAFT_FONTS_DIR, `fonts/${manifest}`, "");
	env.CRAFT_FONTS_REQUIRED = undefined;
	const installedTargets = run("rustup", ["target", "list", "--installed"], {
		encoding: "utf8",
		stdio: "pipe",
	});
	if (!installedTargets.split(/\s+/).includes("wasm32-unknown-unknown"))
		run("rustup", ["target", "add", "wasm32-unknown-unknown"]);
	let localPackages = [];
	if (prepared.reconcileLock) {
		// Cargo may remove dependency edges, but must keep reviewed versions and
		// checksums. Always start from the pin so earlier local builds cannot drift.
		// Allow downloads so reconciliation also works with an empty Cargo cache.
		write("Cargo.lock", original("Cargo.lock"));
		run(
			"cargo",
			[
				"tree",
				"--target",
				"wasm32-unknown-unknown",
				"-p",
				pin.cargoPackage,
				...(prepared.cargoArgs || []),
			],
			{ env, stdio: "pipe" },
		);
		localPackages = assertPinnedLockPackages(
			original("Cargo.lock"),
			readFileSync(join(source, "Cargo.lock"), "utf8"),
			{ source, pinnedSourceOverrides: prepared.pinnedSourceOverrides },
		);
	} else if (prepared.pinnedSourceOverrides?.length) {
		throw new Error("Pinned source overrides require lockfile reconciliation.");
	}
	if (usesPlatformMedia(id)) {
		const dependencies = run(
			"cargo",
			[
				"tree",
				"--locked",
				"--target",
				"wasm32-unknown-unknown",
				"-p",
				pin.cargoPackage,
				"--edges",
				"normal",
				"--prefix",
				"none",
				"--format",
				"{p} features=[{f}]",
				...(prepared.cargoArgs || []),
			],
			{ env, encoding: "utf8", stdio: "pipe" },
		);
		assertPlatformMediaDependencies(
			id,
			dependencies,
			pin.cargoPackage,
			localPackages,
		);
	}
	sourceWriter.finish();
	timings.phase("compile");
	run(
		"cargo",
		[
			"build",
			"--locked",
			...(process.env.CRAFT_BUILD_REPORT_DIR ? ["--timings"] : []),
			"--release",
			"-p",
			pin.cargoPackage,
			"--target",
			"wasm32-unknown-unknown",
			...(prepared.cargoArgs || []),
		],
		{ env },
	);
	timings.phase("package");
	rmSync(output, { recursive: true, force: true });
	mkdirSync(output, { recursive: true });
	run(executable, [
		join(
			cargoTarget,
			`wasm32-unknown-unknown/release/${pin.wasmFile || pin.cargoPackage}.wasm`,
		),
		"--target",
		"web",
		"--out-dir",
		output,
		"--out-name",
		"craft",
	]);
	// Pages limits each static asset to 25 MiB. Gzip also keeps the cold load small;
	// chunks leave room for upstream growth without changing hosting configuration.
	const wasmPath = join(output, "craft_bg.wasm");
	initializeWasmStartup({ id, output, moduleName: "craft" });
	const wasmBytes = readFileSync(wasmPath);
	assertNoBrandArtwork(source, wasmBytes);
	const compressed = gzipSync(wasmBytes, { level: 9 });
	const wasm = {
		compression: "gzip",
		byteLength: wasmBytes.byteLength,
		sha256: digest(wasmPath),
		parts: [],
	};
	const chunkSize = 8 * 1024 * 1024;
	for (let offset = 0; offset < compressed.byteLength; offset += chunkSize) {
		const name = `craft.wasm.gz.${String(wasm.parts.length).padStart(3, "0")}`;
		const bytes = compressed.subarray(offset, offset + chunkSize);
		writeFileSync(join(output, name), bytes);
		wasm.parts.push({
			file: name,
			byteLength: bytes.byteLength,
			sha256: digest(join(output, name)),
		});
	}
	rmSync(wasmPath);
	for (const name of ["host.js", "index.html"])
		cpSync(join(suiteDirectory, name), join(output, name));
	cpSync(join(suiteDirectory, "protocol.mjs"), join(output, "protocol.js"));
	cpSync(join(suiteDirectory, "mcp.mjs"), join(output, "mcp.mjs"));
	for (const asset of prepared.assets || []) {
		mkdirSync(dirname(join(output, asset.to)), { recursive: true });
		cpSync(join(source, asset.from), join(output, asset.to), {
			recursive: true,
		});
	}
	const licenses = join(output, "licenses");
	mkdirSync(licenses, { recursive: true });
	const attributionFiles = run(
		"git",
		["ls-tree", "-r", "--name-only", pin.revision],
		{ encoding: "utf8", stdio: "pipe" },
	)
		.trim()
		.split("\n")
		.filter((name) =>
			/(^|\/)(LICENSE[^/]*|NOTICE[^/]*|COPYING[^/]*|OFL[^/]*|ATTRIBUTION\.md|CGATS_LICENSE\.txt)$/i.test(
				name,
			),
		);
	for (const name of attributionFiles) {
		const destination = join(licenses, name.replace(/\.md$/, ".txt"));
		mkdirSync(dirname(destination), { recursive: true });
		writeFileSync(destination, original(name));
	}
	bundleLicenses({
		source,
		output,
		pin,
		cargoPackage: pin.cargoPackage,
		cargoArgs: prepared.cargoArgs || [],
		sourceOverrides: localPackages,
		env,
	});
	const files = {};
	for (const name of readdirSync(output, { recursive: true })) {
		if (name === "runtime.json") continue;
		try {
			if (readFileSync(join(output, name)).byteLength > 25 * 1024 * 1024)
				throw new Error(
					`${pin.label} asset ${name} exceeds the static hosting limit of 25 MiB.`,
				);
			files[name] = digest(join(output, name));
		} catch (error) {
			if (error.code !== "EISDIR") throw error;
		}
	}
	writeFileSync(
		stampPath,
		`${JSON.stringify(
			{
				...pin,
				id,
				maxBytes: pin.maxBytes || 200 * 1024 * 1024,
				fingerprint,
				lockSha256: digest(join(source, "Cargo.lock")),
				wasm,
				files,
			},
			null,
			2,
		)}\n`,
	);
}
timings.phase("stage");
if (!skipStage) stage();
else console.log(`${pin.label} prepared without staging public assets.`);
const built = JSON.parse(readFileSync(stampPath, "utf8"));
timings.finish({
	cached: cacheHit,
	success: true,
	cacheMode: buildCache.mode,
	cargoTarget: buildCache.cargoTarget,
	revision: pin.revision,
	staged: !skipStage,
	wasmBytes: built.wasm.byteLength,
	compressedBytes: built.wasm.parts.reduce(
		(sum, part) => sum + part.byteLength,
		0,
	),
});
succeeded = true;
releaseLock();
process.removeListener("exit", releaseLock);
