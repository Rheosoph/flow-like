import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	closeSync,
	cpSync,
	existsSync,
	mkdirSync,
	openSync,
	readFileSync,
	readdirSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { ensureWasmBindgen } from "../craft-viewers/build-tools.mjs";
import {
	assertNoBrandArtwork,
	bundleLicenses,
	hashLicenseInputs,
} from "../craft-viewers/licenses.mjs";
import { prepareLiveMcp } from "../craft-viewers/live-mcp.mjs";
import { wasmLinkerEnvironment } from "../device-crypto/build-web.mjs";

const directory = dirname(fileURLToPath(import.meta.url));
const root = resolve(directory, "../..");
const toolchain =
	process.env.RUSTUP_TOOLCHAIN ||
	readFileSync(join(root, "rust-toolchain.toml"), "utf8").match(
		/^channel\s*=\s*"([^"]+)"/m,
	)?.[1];
if (!toolchain)
	throw new Error("The repository Rust toolchain is not configured.");
const buildEnvironment = { ...process.env, RUSTUP_TOOLCHAIN: toolchain };
const pin = JSON.parse(readFileSync(join(directory, "upstream.json"), "utf8"));
if (!/^[a-f0-9]{40}$/.test(pin.revision))
	throw new Error("WordCraft must be pinned to an exact Git commit.");
const cache = join(root, "target/wordcraft");
const source =
	process.env.WORDCRAFT_SOURCE || join(cache, "source", pin.revision);
const output = join(cache, "runtime");
const inputs = [
	"upstream.json",
	"host.rs",
	"../craft-viewers/live-mcp.mjs",
	"../craft-viewers/live-mcp.rs",
	"../craft-viewers/mcp-policy.rs",
	"../craft-viewers/mcp.mjs",
	"host.js",
	"protocol.mjs",
	"index.html",
	"build-web.mjs",
];
const hash = createHash("sha256");
hash.update(toolchain);
for (const name of inputs)
	hash.update(name).update(readFileSync(join(directory, name)));
for (const name of [
	"packages/craft-viewers/build-tools.mjs",
	"packages/craft-viewers/build-lock.mjs",
	"packages/device-crypto/build-web.mjs",
	"rust-toolchain.toml",
])
	hash.update(name).update(readFileSync(join(root, name)));
hashLicenseInputs(hash);
const fingerprint = hash.digest("hex");
const stampPath = join(output, "runtime.json");
const targets = ["apps/web/public/wordcraft", "apps/desktop/public/wordcraft"];

function acquireBuildLock() {
	mkdirSync(cache, { recursive: true });
	const lock = join(cache, "build.lock");
	const deadline = Date.now() + 10 * 60 * 1000;
	let waiting = false;
	while (true) {
		try {
			const descriptor = openSync(lock, "wx");
			writeFileSync(descriptor, String(process.pid));
			closeSync(descriptor);
			let released = false;
			const release = () => {
				if (released) return;
				released = true;
				try {
					if (readFileSync(lock, "utf8") === String(process.pid))
						rmSync(lock, { force: true });
				} catch (error) {
					if (error.code !== "ENOENT") throw error;
				}
			};
			process.on("exit", release);
			return release;
		} catch (error) {
			if (error.code !== "EEXIST") throw error;
			try {
				const owner = Number(readFileSync(lock, "utf8"));
				if (Number.isSafeInteger(owner) && owner > 0) {
					try {
						process.kill(owner, 0);
					} catch (error) {
						if (error.code === "ESRCH") {
							rmSync(lock, { force: true });
							continue;
						}
					}
				}
			} catch (error) {
				if (error.code === "ENOENT") continue;
				throw error;
			}
			if (Date.now() >= deadline)
				throw new Error(
					`Another WordCraft build is still running. Wait for it to finish, then retry. Lock: ${lock}`,
				);
			if (!waiting) {
				console.log("Waiting for the other WordCraft build…");
				waiting = true;
			}
			Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250);
		}
	}
}

const releaseLock = acquireBuildLock();

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
	});
}

function replaceOnce(text, search, replacement, name) {
	if (text.split(search).length !== 2) {
		throw new Error(
			`WordCraft ${pin.revision} changed ${name}; review the host adapter before upgrading.`,
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
		`WordCraft ${pin.version} (${pin.revision.slice(0, 12)}) ready for web and desktop.`,
	);
}

if (!cached()) {
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
	if (revision !== pin.revision && process.env.WORDCRAFT_SOURCE && revision) {
		throw new Error(`WORDCRAFT_SOURCE must be checked out at ${pin.revision}.`);
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

	// Keep the adapter in this package. Upgrades only change upstream.json; changed
	// integration seams fail here or during compilation instead of silently applying.
	const patch = (path, search, replacement, label) =>
		writeFileSync(
			join(source, path),
			replaceOnce(original(path), search, replacement, label),
		);
	patch(
		"crates/ui-egui/src/services.rs",
		"pub struct Services {",
		"pub struct Services {\n    pub host_command: Option<Box<dyn Fn(&str) -> Option<Result<serde_json::Value, String>>>>,",
		"Services",
	);
	const commandsPath = "crates/ui-egui/src/app/commands.rs";
	let commands = original(commandsPath);
	for (const method of ["run", "execute"]) {
		const signature = `pub fn ${method}(&mut self, id: &str, params: Value) -> Result<Value, String> {`;
		commands = replaceOnce(
			commands,
			signature,
			`${signature}\n        if let Some(result) = self.services.host_command.as_ref().and_then(|filter| filter(id)) { return result; }`,
			`WordApp::${method}`,
		);
	}
	writeFileSync(join(source, commandsPath), commands);
	const framePath = "crates/ui-egui/src/app/frame.rs";
	let frame = original(framePath);
	if (frame.split("chrome::title_bar(self, ui);").length !== 3)
		throw new Error("Review WordCraft title bars.");
	frame = frame.replaceAll(
		"chrome::title_bar(self, ui);",
		"if self.services.host_command.is_none() { chrome::title_bar(self, ui); }",
	);
	writeFileSync(join(source, framePath), frame);
	writeFileSync(
		join(source, "crates/ui-egui/src/windows.rs"),
		original("crates/ui-egui/src/windows.rs")
			.replaceAll(" - WordCraft", " - Flows Word")
			.replaceAll('"WordCraft"', '"Flows Word"'),
	);
	writeFileSync(
		join(source, "crates/ui-egui/src/lib.rs"),
		`${original("crates/ui-egui/src/lib.rs")}\nimpl WordApp {\n    pub fn host_has_drafts(&self) -> bool { panes::has_comment_drafts(self) }\n    pub fn host_flush_drafts(&mut self) -> Result<(), String> { panes::flush_comment_drafts(self) }\n}\n`,
	);
	const dialogsPath = "crates/ui-egui/src/dialogs/dispatch.rs";
	let dialogs = original(dialogsPath);
	for (const [search, replacement] of [
		[
			'egui::RichText::new(tl!("WordCraft")).font(semibold(22.0))',
			'egui::RichText::new("Flows Word").font(semibold(22.0))',
		],
		[
			'"A free, open-source word processor written from scratch in Rust.\\nPart of the Crafting Apps from the ArtCraft team."',
			'"Document viewing and editing in Flow-Like, powered by WordCraft."',
		],
	]) {
		dialogs = replaceOnce(dialogs, search, replacement, "product identity");
	}
	writeFileSync(join(source, dialogsPath), dialogs);
	const ribbonPath = "crates/ui-egui/src/ribbon.rs";
	let ribbon = original(ribbonPath);
	ribbon = replaceOnce(
		ribbon,
		'let mut tabs: Vec<(String, bool)> = vec![("File".to_string(), false)];',
		'let mut tabs: Vec<(String, bool)> = if app.services.host_command.is_some() { vec![] } else { vec![("File".to_string(), false)] };',
		"File tab",
	);
	writeFileSync(join(source, ribbonPath), ribbon);
	let dialogChrome = original("crates/ui-egui/src/dialogs/chrome.rs");
	dialogChrome = replaceOnce(
		dialogChrome,
		'Dialog::About { .. } => "About WordCraft",',
		'Dialog::About { .. } => "About Flows Word",',
		"About title",
	);
	dialogChrome = replaceOnce(
		dialogChrome,
		'Dialog::SaveChanges { .. } => "WordCraft",',
		'Dialog::SaveChanges { .. } => "Flows Word",',
		"Save title",
	);
	writeFileSync(
		join(source, "crates/ui-egui/src/dialogs/chrome.rs"),
		dialogChrome,
	);
	const manifestPath = "apps/wordcraft-web/Cargo.toml";
	writeFileSync(
		join(source, manifestPath),
		replaceOnce(
			original(manifestPath),
			"wordcraft-engine = { workspace = true }",
			"wordcraft-engine = { workspace = true }\nwordcraft-render = { workspace = true }",
			"web dependencies",
		),
	);
	const lock = original("Cargo.lock");
	const webPackage =
		/\[\[package\]\]\nname = "wordcraft-web"\n[\s\S]*?(?=\n\[\[package\]\]|$)/;
	const packageBlock = lock.match(webPackage)?.[0];
	if (!packageBlock)
		throw new Error("Upstream Cargo.lock does not include wordcraft-web.");
	writeFileSync(
		join(source, "Cargo.lock"),
		replaceOnce(
			lock,
			packageBlock,
			replaceOnce(
				packageBlock,
				' "wordcraft-engine",',
				' "wordcraft-engine",\n "wordcraft-render",',
				"locked web dependencies",
			),
			"Cargo.lock",
		),
	);
	const lockedBindgen = lock.match(
		/\[\[package\]\]\nname = "wasm-bindgen"\nversion = "([^"]+)"/,
	)?.[1];
	if (lockedBindgen !== pin.wasmBindgen)
		throw new Error(
			`Set upstream.json wasmBindgen to the pinned lock version ${lockedBindgen}.`,
		);
	cpSync(
		join(directory, "host.rs"),
		join(source, "apps/wordcraft-web/src/web.rs"),
	);
	writeFileSync(
		join(source, "apps/wordcraft-web/src/main.rs"),
		'#[cfg(target_arch = "wasm32")]\nmod web;\n#[cfg(target_arch = "wasm32")]\nfn main() { web::start(); }\n#[cfg(not(target_arch = "wasm32"))]\nfn main() {}\n',
	);

	prepareLiveMcp({
		id: "wordcraft",
		source,
		original,
		replaceOnce,
		write: (path, text) => writeFileSync(join(source, path), text),
	});

	const executable = ensureWasmBindgen({
		root,
		version: pin.wasmBindgen,
		env: buildEnvironment,
		run,
	});
	const cargoTarget = process.env.WORDCRAFT_TARGET_DIR || join(cache, "cargo");
	const env = {
		...buildEnvironment,
		...wasmLinkerEnvironment({
			directory: join(cache, "linker"),
			env: buildEnvironment,
		}),
		CARGO_TARGET_DIR: cargoTarget,
		// Release jobs pass native linker shims through these. They cannot be linked
		// into WebAssembly; the explicit --target below also overrides CARGO_BUILD_TARGET.
		RUSTFLAGS: undefined,
		CARGO_ENCODED_RUSTFLAGS: undefined,
	};
	// Optional system font checkouts would change the runtime without changing its
	// pin or cache key. This build uses only upstream's bundled, attributed fonts.
	env.CRAFT_FONTS_DIR = undefined;
	env.CRAFT_FONTS_REQUIRED = undefined;
	const installedTargets = run("rustup", ["target", "list", "--installed"], {
		encoding: "utf8",
		stdio: "pipe",
	});
	if (!installedTargets.split(/\s+/).includes("wasm32-unknown-unknown"))
		run("rustup", ["target", "add", "wasm32-unknown-unknown"]);
	run(
		"cargo",
		[
			"build",
			"--locked",
			"--release",
			"-p",
			"wordcraft-web",
			"--target",
			"wasm32-unknown-unknown",
		],
		{ env },
	);
	rmSync(output, { recursive: true, force: true });
	mkdirSync(output, { recursive: true });
	run(executable, [
		join(cargoTarget, "wasm32-unknown-unknown/release/wordcraft-web.wasm"),
		"--target",
		"web",
		"--out-dir",
		output,
		"--out-name",
		"wordcraft",
	]);
	// Pages limits each static asset to 25 MiB. Gzip also keeps the cold load small;
	// chunks leave room for upstream growth without changing hosting configuration.
	const wasmPath = join(output, "wordcraft_bg.wasm");
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
		const name = `wordcraft.wasm.gz.${String(wasm.parts.length).padStart(3, "0")}`;
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
		cpSync(join(directory, name), join(output, name));
	cpSync(join(directory, "protocol.mjs"), join(output, "protocol.js"));
	cpSync(join(directory, "../craft-viewers/mcp.mjs"), join(output, "mcp.mjs"));
	const licenses = join(output, "licenses");
	mkdirSync(licenses, { recursive: true });
	for (const name of [
		"LICENSE-MIT",
		"LICENSE-APACHE",
		"NOTICE",
		"ATTRIBUTION.md",
	]) {
		if (existsSync(join(source, name)))
			cpSync(join(source, name), join(licenses, name.replace(/\.md$/, ".txt")));
	}
	for (const name of readdirSync(join(source, "assets/fonts")).filter((name) =>
		name.endsWith(".txt"),
	))
		cpSync(join(source, "assets/fonts", name), join(licenses, name));
	bundleLicenses({ source, output, pin, cargoPackage: "wordcraft-web", env });
	const files = {};
	for (const name of readdirSync(output, { recursive: true })) {
		if (name === "runtime.json") continue;
		try {
			if (readFileSync(join(output, name)).byteLength > 25 * 1024 * 1024)
				throw new Error(
					`WordCraft asset ${name} exceeds the static hosting limit of 25 MiB.`,
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
stage();
releaseLock();
process.removeListener("exit", releaseLock);
