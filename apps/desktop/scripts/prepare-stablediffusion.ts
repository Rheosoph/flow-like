/** Prepare a pinned sd-server runtime without sharing llama.cpp libraries. */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const TAG = "master-841-6b3edaa";
const COMMIT = "6b3edaaf32cc19e5bb2d819c788bd557eddc8eba";
const ROOT = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../src-tauri/runtimes/stablediffusion",
);
const MACOS_TARGET = "14.0";
const ASSETS = {
	"win-x64": {
		name: "sd-master-6b3edaa-bin-win-vulkan-x64.zip",
		sha256: "b8640b12fd708a2d26a5e31d73861a50ede73f1b3d6132a4dbe037692b68c0f2",
	},
} as const;

export function hostPlatform(): string {
	if (process.platform === "darwin")
		return process.arch === "arm64" ? "mac-arm" : "mac-intel";
	if (process.platform === "win32" && process.arch === "x64") return "win-x64";
	if (process.platform === "linux" && process.arch === "x64")
		return "linux-x64";
	throw new Error(
		"No bundled sd-server for this host. Use an existing sd-server endpoint or FLOW_LIKE_SD_SERVER with a source build.",
	);
}

// Extract only runtime files. Flattening gives the executable and its private
// libraries a common loader directory, regardless of the release archive layout.
export function extractRuntime(
	archive: Buffer,
	destination: string,
	executable: string,
) {
	const temporary = fs.mkdtempSync(
		path.join(os.tmpdir(), "flow-like-sd-archive-"),
	);
	const names = new Set<string>();
	try {
		const archivePath = path.join(temporary, "runtime.zip");
		const extracted = path.join(temporary, "extracted");
		fs.writeFileSync(archivePath, archive);
		fs.mkdirSync(extracted);
		if (process.platform === "win32")
			execFileSync("tar", ["-xf", archivePath, "-C", extracted]);
		else execFileSync("unzip", ["-q", archivePath, "-d", extracted]);
		const collect = (directory: string) => {
			for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
				const source = path.join(directory, entry.name);
				if (entry.isSymbolicLink())
					throw new Error(
						`Runtime archive contains a symbolic link: ${entry.name}`,
					);
				if (entry.isDirectory()) {
					collect(source);
					continue;
				}
				const name = entry.name;
				if (
					!entry.isFile() ||
					(name !== executable &&
						!/\.(dll|dylib|so(\.\d+)*|txt|metal|metallib)$/i.test(name))
				)
					continue;
				if (names.has(name.toLowerCase()))
					throw new Error(`Duplicate runtime file in archive: ${name}`);
				names.add(name.toLowerCase());
				fs.copyFileSync(source, path.join(destination, name));
			}
		};
		collect(extracted);
	} finally {
		fs.rmSync(temporary, { recursive: true, force: true });
	}
	if (!names.has(executable.toLowerCase()))
		throw new Error(`Release archive does not contain ${executable}`);
	fs.chmodSync(path.join(destination, executable), 0o755);
}

function run(command: string, args: string[], cwd?: string) {
	execFileSync(command, args, { cwd, stdio: "inherit" });
}

async function downloadRuntime(
	platform: keyof typeof ASSETS,
	destination: string,
) {
	const asset = ASSETS[platform];
	const response = await fetch(
		`https://github.com/leejet/stable-diffusion.cpp/releases/download/${TAG}/${asset.name}`,
	);
	if (!response.ok) throw new Error(`Download failed: HTTP ${response.status}`);
	const archive = Buffer.from(await response.arrayBuffer());
	if (createHash("sha256").update(archive).digest("hex") !== asset.sha256) {
		throw new Error("sd-server archive checksum mismatch");
	}
	extractRuntime(
		archive,
		destination,
		platform === "win-x64" ? "sd-server.exe" : "sd-server",
	);
}

function buildSourceRuntime(
	platform: string,
	destination: string,
	executable: string,
) {
	const mac = platform.startsWith("mac-");
	if (mac && process.platform !== "darwin")
		throw new Error("Build macOS sd-server on macOS.");
	if (!mac && process.platform !== "linux")
		throw new Error("Build Linux sd-server on Linux.");
	// The upstream macOS archive requires macOS 26. Build the same revision for
	// Flow-Like's macOS 14 minimum instead of shipping an incompatible binary.
	const source = path.join(ROOT, ".source");
	if (!fs.existsSync(path.join(source, ".git"))) {
		run("git", [
			"clone",
			"--depth",
			"1",
			"--branch",
			TAG,
			"https://github.com/leejet/stable-diffusion.cpp.git",
			source,
		]);
	}
	const head = execFileSync("git", ["rev-parse", "HEAD"], {
		cwd: source,
		encoding: "utf8",
	}).trim();
	if (head !== COMMIT)
		throw new Error(
			`Unexpected source revision ${head}; remove ${source} and retry.`,
		);
	run(
		"git",
		[
			"submodule",
			"update",
			"--init",
			"--recursive",
			"--depth",
			"1",
			"--",
			"ggml",
			"thirdparty/libwebp",
			"thirdparty/libwebm",
		],
		source,
	);
	const build = path.join(ROOT, `.build-${platform}`);
	const targetFlags = mac
		? [
				`-DCMAKE_OSX_DEPLOYMENT_TARGET=${MACOS_TARGET}`,
				`-DCMAKE_OSX_ARCHITECTURES=${platform === "mac-arm" ? "arm64" : "x86_64"}`,
				"-DSD_METAL=ON",
			]
		: ["-DSD_VULKAN=ON"];
	run("cmake", [
		"-S",
		source,
		"-B",
		build,
		"-DCMAKE_BUILD_TYPE=Release",
		...targetFlags,
		"-DSD_BUILD_SHARED_LIBS=OFF",
		"-DSD_BUILD_EXAMPLES=ON",
		"-DSD_SERVER_BUILD_FRONTEND=OFF",
		"-DGGML_NATIVE=OFF",
	]);
	run("cmake", [
		"--build",
		build,
		"--config",
		"Release",
		"--target",
		"sd-server",
		"--parallel",
		String(Math.min(os.availableParallelism(), 8)),
	]);
	fs.copyFileSync(
		path.join(build, "bin/sd-server"),
		path.join(destination, executable),
	);
	fs.chmodSync(path.join(destination, executable), 0o755);
	for (const [sourceFile, name] of [
		["LICENSE", "stable-diffusion.cpp.txt"],
		["ggml/LICENSE", "ggml.txt"],
		["thirdparty/libwebp/COPYING", "webp-COPYING.txt"],
		["thirdparty/libwebp/PATENTS", "webp-PATENTS.txt"],
		["thirdparty/libwebp/AUTHORS", "webp-AUTHORS.txt"],
		["thirdparty/libwebm/LICENSE.TXT", "webm-LICENSE.txt"],
		["thirdparty/libwebm/PATENTS.TXT", "webm-PATENTS.txt"],
		["thirdparty/libwebm/AUTHORS.TXT", "webm-AUTHORS.txt"],
	]) {
		fs.copyFileSync(
			path.join(source, sourceFile),
			path.join(destination, name),
		);
	}
}

export async function prepare(platform: string, force = false) {
	if (!["mac-arm", "mac-intel", "linux-x64", "win-x64"].includes(platform)) {
		throw new Error(`Unsupported sd-server platform: ${platform}`);
	}
	const destination = path.join(ROOT, platform);
	const executable =
		platform === "mac-arm"
			? "sd-server-aarch64-apple-darwin"
			: platform === "mac-intel"
				? "sd-server-x86_64-apple-darwin"
				: platform === "win-x64"
					? "sd-server.exe"
					: "sd-server";
	const stamp = {
		recipe: 2,
		tag: TAG,
		commit: COMMIT,
		platform,
		macosTarget: platform.startsWith("mac-") ? MACOS_TARGET : null,
	};
	const manifest = path.join(destination, "runtime.json");
	if (!force && fs.existsSync(manifest)) {
		try {
			const installed = JSON.parse(fs.readFileSync(manifest, "utf8"));
			const files: Record<string, string> = installed.files ?? {};
			if (
				JSON.stringify(installed.build) === JSON.stringify(stamp) &&
				executable in files &&
				Object.entries(files).every(
					([name, checksum]) =>
						path.basename(name) === name &&
						fs.existsSync(path.join(destination, name)) &&
						createHash("sha256")
							.update(fs.readFileSync(path.join(destination, name)))
							.digest("hex") === checksum,
				)
			) {
				fs.chmodSync(path.join(destination, executable), 0o755);
				return;
			}
		} catch {
			// Rebuild an incomplete or unreadable cached installation.
		}
	}
	fs.mkdirSync(ROOT, { recursive: true });
	const staging = fs.mkdtempSync(path.join(ROOT, `.staging-${platform}-`));
	try {
		if (platform !== "win-x64")
			buildSourceRuntime(platform, staging, executable);
		else await downloadRuntime(platform as keyof typeof ASSETS, staging);
		const files = Object.fromEntries(
			fs
				.readdirSync(staging)
				.sort()
				.map((name) => [
					name,
					createHash("sha256")
						.update(fs.readFileSync(path.join(staging, name)))
						.digest("hex"),
				]),
		);
		fs.writeFileSync(
			path.join(staging, "runtime.json"),
			`${JSON.stringify({ build: stamp, files }, null, 2)}\n`,
		);
		// Replace only this runtime after verification. The llama updater owns a
		// different directory and cannot erase these files.
		const previous = `${destination}.previous`;
		fs.rmSync(previous, { recursive: true, force: true });
		if (fs.existsSync(destination)) fs.renameSync(destination, previous);
		try {
			fs.renameSync(staging, destination);
		} catch (error) {
			if (fs.existsSync(previous)) fs.renameSync(previous, destination);
			throw error;
		}
		fs.rmSync(previous, { recursive: true, force: true });
		console.log(`Prepared stablediffusion.cpp ${TAG} for ${platform}`);
	} finally {
		fs.rmSync(staging, { recursive: true, force: true });
	}
}

if (import.meta.main) {
	const args = process.argv.slice(2);
	if (args.includes("--help")) {
		console.log(
			"bun scripts/prepare-stablediffusion.ts [--platform mac-arm|mac-intel|linux-x64|win-x64] [--force]",
		);
	} else {
		const index = args.indexOf("--platform");
		if (index >= 0 && !args[index + 1])
			throw new Error("--platform requires a value");
		await prepare(
			index >= 0 ? args[index + 1] : hostPlatform(),
			args.includes("--force"),
		);
	}
}
