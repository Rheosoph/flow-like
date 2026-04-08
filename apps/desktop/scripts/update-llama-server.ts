/**
 * Downloads the latest (or pinned) llama.cpp release binaries for all supported platforms
 * and places them in the correct Tauri binaries directories with proper naming.
 *
 * Usage:
 *   bun run scripts/update-llama-server.ts                  # uses pinned version
 *   bun run scripts/update-llama-server.ts --latest         # fetches latest release
 *   bun run scripts/update-llama-server.ts --tag b8660      # fetches specific tag
 *   bun run scripts/update-llama-server.ts --platform mac-arm  # single platform
 *
 * Environment:
 *   GITHUB_TOKEN  — optional, avoids rate limits
 */

import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const PINNED_TAG = "b8660";
const GITHUB_API = "https://api.github.com";
const OWNER = "ggml-org";
const REPO = "llama.cpp";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const BINARIES_DIR = path.resolve(SCRIPT_DIR, "../src-tauri/binaries");

// Tauri externalBin requires a target-triple suffix on macOS/Linux executables and
// macOS dylibs. Windows DLLs listed as resources don't need a suffix.
interface PlatformConfig {
	/** Key used in release asset name */
	assetName: string;
	/** Archive type */
	archiveType: "tar.gz" | "zip";
	/** Output directory relative to BINARIES_DIR */
	outDir: string;
	/** Tauri target triple for binary suffixing */
	tauriTriple: string;
	/** Files to extract and how to rename them (release name → local name with Tauri suffix) */
	files: FileMapping[];
}

interface FileMapping {
	/** Filename inside the release archive (under the llama-bXXXX/ prefix) */
	src: string;
	/** Destination filename. Use {TRIPLE} as placeholder for tauriTriple. */
	dst: string;
	/** If true, make executable (chmod +x) */
	executable?: boolean;
}

function macDylib(name: string): FileMapping {
	return { src: `${name}.dylib`, dst: `${name}.dylib-{TRIPLE}` };
}

function linuxSo(name: string): FileMapping {
	return { src: `${name}.so`, dst: `${name}.so` };
}

function winDll(name: string): FileMapping {
	return { src: `${name}.dll`, dst: `${name}.dll` };
}

function winDllSuffixed(name: string): FileMapping {
	return { src: `${name}.dll`, dst: `${name}.dll-{TRIPLE}` };
}

const PLATFORMS: Record<string, PlatformConfig> = {
	"mac-arm": {
		assetName: "llama-{TAG}-bin-macos-arm64.tar.gz",
		archiveType: "tar.gz",
		outDir: "mac/arm",
		tauriTriple: "aarch64-apple-darwin",
		files: [
			{ src: "llama-server", dst: "llama-server-{TRIPLE}", executable: true },
			macDylib("libllama"),
			macDylib("libggml"),
			macDylib("libggml-base"),
			macDylib("libggml-blas"),
			macDylib("libggml-cpu"),
			macDylib("libggml-metal"),
			macDylib("libggml-rpc"),
			macDylib("libmtmd"),
		],
	},
	"mac-intel": {
		assetName: "llama-{TAG}-bin-macos-x64.tar.gz",
		archiveType: "tar.gz",
		outDir: "mac/intel",
		tauriTriple: "x86_64-apple-darwin",
		files: [
			{ src: "llama-server", dst: "llama-server-{TRIPLE}", executable: true },
			macDylib("libllama"),
			macDylib("libggml"),
			macDylib("libggml-base"),
			macDylib("libggml-blas"),
			macDylib("libggml-cpu"),
			macDylib("libggml-rpc"),
			macDylib("libmtmd"),
		],
	},
	"win-x64": {
		assetName: "llama-{TAG}-bin-win-vulkan-x64.zip",
		archiveType: "zip",
		outDir: "win/x64",
		tauriTriple: "x86_64-pc-windows-msvc",
		files: [
			{
				src: "llama-server.exe",
				dst: "llama-server-{TRIPLE}.exe",
				executable: true,
			},
			winDll("llama"),
			winDll("mtmd"),
			winDll("ggml"),
			winDll("ggml-base"),
			winDll("ggml-rpc"),
			winDll("ggml-vulkan"),
			winDll("ggml-cpu-alderlake"),
			winDll("ggml-cpu-cannonlake"),
			winDll("ggml-cpu-cascadelake"),
			winDll("ggml-cpu-cooperlake"),
			winDll("ggml-cpu-haswell"),
			winDll("ggml-cpu-icelake"),
			winDll("ggml-cpu-ivybridge"),
			winDll("ggml-cpu-piledriver"),
			winDll("ggml-cpu-sandybridge"),
			winDll("ggml-cpu-sapphirerapids"),
			winDll("ggml-cpu-skylakex"),
			winDll("ggml-cpu-sse42"),
			winDll("ggml-cpu-x64"),
			winDll("ggml-cpu-zen4"),
			winDll("libomp140.x86_64"),
		],
	},
	"win-arm": {
		assetName: "llama-{TAG}-bin-win-cpu-arm64.zip",
		archiveType: "zip",
		outDir: "win/arm",
		tauriTriple: "aarch64-pc-windows-msvc",
		files: [
			{
				src: "llama-server.exe",
				dst: "llama-server.exe-{TRIPLE}",
				executable: true,
			},
			winDllSuffixed("llama"),
			winDllSuffixed("mtmd"),
			winDllSuffixed("ggml"),
			winDllSuffixed("ggml-base"),
			winDllSuffixed("ggml-cpu"),
			winDllSuffixed("ggml-rpc"),
			winDllSuffixed("libomp140.aarch64"),
		],
	},
	"linux-x64": {
		assetName: "llama-{TAG}-bin-ubuntu-vulkan-x64.tar.gz",
		archiveType: "tar.gz",
		outDir: "linux/x64",
		tauriTriple: "x86_64-unknown-linux-gnu",
		files: [
			{
				src: "llama-server",
				dst: "llama-server-{TRIPLE}",
				executable: true,
			},
			linuxSo("libllama"),
			linuxSo("libmtmd"),
			linuxSo("libggml"),
			linuxSo("libggml-base"),
			linuxSo("libggml-rpc"),
			linuxSo("libggml-vulkan"),
			linuxSo("libggml-cpu-alderlake"),
			linuxSo("libggml-cpu-cannonlake"),
			linuxSo("libggml-cpu-cascadelake"),
			linuxSo("libggml-cpu-cooperlake"),
			linuxSo("libggml-cpu-haswell"),
			linuxSo("libggml-cpu-icelake"),
			linuxSo("libggml-cpu-ivybridge"),
			linuxSo("libggml-cpu-piledriver"),
			linuxSo("libggml-cpu-sandybridge"),
			linuxSo("libggml-cpu-sapphirerapids"),
			linuxSo("libggml-cpu-skylakex"),
			linuxSo("libggml-cpu-sse42"),
			linuxSo("libggml-cpu-x64"),
			linuxSo("libggml-cpu-zen4"),
		],
	},
};

function getHeaders(): Record<string, string> {
	const headers: Record<string, string> = {
		Accept: "application/vnd.github.v3+json",
		"User-Agent": "flow-like-llama-updater",
	};
	if (process.env.GITHUB_TOKEN) {
		headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
	}
	return headers;
}

async function resolveTag(requested: string | "latest"): Promise<string> {
	if (requested !== "latest") return requested;

	const url = `${GITHUB_API}/repos/${OWNER}/${REPO}/releases/latest`;
	const resp = await fetch(url, { headers: getHeaders() });
	if (!resp.ok) throw new Error(`Failed to fetch latest release: ${resp.status}`);
	const data = (await resp.json()) as { tag_name: string };
	return data.tag_name;
}

async function downloadArchive(url: string, dest: string): Promise<void> {
	console.log(`  Downloading ${url}`);
	const resp = await fetch(url, {
		headers: getHeaders(),
		redirect: "follow",
	});
	if (!resp.ok) throw new Error(`Download failed: ${resp.status} ${url}`);
	const buffer = await resp.arrayBuffer();
	fs.writeFileSync(dest, Buffer.from(buffer));
}

function extractFiles(
	archivePath: string,
	archiveType: "tar.gz" | "zip",
	fileMap: Map<string, string>,
	outDir: string,
): void {
	const tmpDir = fs.mkdtempSync(path.join(outDir, ".extract-"));

	try {
		if (archiveType === "tar.gz") {
			execSync(`tar -xzf "${archivePath}" -C "${tmpDir}"`, {
				stdio: "pipe",
			});
		} else {
			execSync(`unzip -o "${archivePath}" -d "${tmpDir}"`, {
				stdio: "pipe",
			});
		}

		// Find the extracted directory (usually llama-bXXXX/)
		const entries = fs.readdirSync(tmpDir);
		let extractRoot = tmpDir;
		for (const entry of entries) {
			const full = path.join(tmpDir, entry);
			if (fs.statSync(full).isDirectory()) {
				extractRoot = full;
				break;
			}
		}

		for (const [srcName, dstName] of fileMap) {
			const srcPath = path.join(extractRoot, srcName);
			const dstPath = path.join(outDir, dstName);

			if (!fs.existsSync(srcPath)) {
				console.warn(`  ⚠ Missing in archive: ${srcName}`);
				continue;
			}

			fs.copyFileSync(srcPath, dstPath);
			console.log(`  ✓ ${dstName}`);
		}
	} finally {
		fs.rmSync(tmpDir, { recursive: true, force: true });
	}
}

function fixMacOsDylibNames(
	outDir: string,
	config: PlatformConfig,
): void {
	if (!config.tauriTriple.includes("apple-darwin")) return;
	if (process.platform !== "darwin") return;

	const triple = config.tauriTriple;
	const machOPaths = config.files
		.filter((f) => f.executable || f.src.endsWith(".dylib"))
		.map((f) => path.join(outDir, f.dst.replace("{TRIPLE}", triple)))
		.filter((filePath) => fs.existsSync(filePath));
	const dependencyMappings = new Map<string, string>();

	for (const fm of config.files) {
		if (!fm.src.endsWith(".dylib")) continue;
		const dstPath = path.join(outDir, fm.dst.replace("{TRIPLE}", triple));
		if (!fs.existsSync(dstPath)) continue;

		const otoolOut = execSync(`otool -D "${dstPath}"`, {
			encoding: "utf-8",
		});
		const lines = otoolOut.trim().split("\n");
		if (lines.length < 2) continue;
		const currentId = lines[1].trim();
		const desiredId = `@rpath/${path.basename(fm.src)}`;
		const legacyId = desiredId.replace(/\.dylib$/, ".0.dylib");

		dependencyMappings.set(currentId, desiredId);
		if (legacyId !== desiredId) {
			dependencyMappings.set(legacyId, desiredId);
		}

		if (currentId !== desiredId) {
			execSync(
				`install_name_tool -id "${desiredId}" "${dstPath}"`,
				{ stdio: "pipe" },
			);
			console.log(`  🔧 Fixed dylib id: ${currentId} → ${desiredId}`);
		}
	}

	for (const machOPath of machOPaths) {
		const linkedLibraries = execSync(`otool -L "${machOPath}"`, {
			encoding: "utf-8",
		})
			.split("\n")
			.slice(1)
			.map((line) => line.trim().split(" ")[0])
			.filter(Boolean);

		for (const [oldName, newName] of dependencyMappings) {
			if (oldName === newName || !linkedLibraries.includes(oldName)) continue;
			execSync(
				`install_name_tool -change "${oldName}" "${newName}" "${machOPath}"`,
				{ stdio: "pipe" },
			);
			console.log(`  🔗 Fixed dependency in ${path.basename(machOPath)}: ${oldName} → ${newName}`);
		}
	}
}

function cleanDirectory(dir: string): void {
	if (!fs.existsSync(dir)) {
		fs.mkdirSync(dir, { recursive: true });
		return;
	}
	for (const file of fs.readdirSync(dir)) {
		if (file === ".DS_Store") continue;
		const full = path.join(dir, file);
		if (fs.statSync(full).isFile()) {
			fs.unlinkSync(full);
		}
	}
}

async function updatePlatform(
	platformKey: string,
	config: PlatformConfig,
	tag: string,
): Promise<void> {
	const assetName = config.assetName.replace("{TAG}", tag);
	const downloadUrl = `https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${assetName}`;
	const outDir = path.join(BINARIES_DIR, config.outDir);
	const archivePath = path.join(outDir, `_download.${config.archiveType === "tar.gz" ? "tar.gz" : "zip"}`);

	console.log(`\n[${platformKey}] → ${assetName}`);

	// Build file mapping
	const fileMap = new Map<string, string>();
	for (const fm of config.files) {
		fileMap.set(fm.src, fm.dst.replace("{TRIPLE}", config.tauriTriple));
	}

	cleanDirectory(outDir);
	await downloadArchive(downloadUrl, archivePath);
	extractFiles(archivePath, config.archiveType, fileMap, outDir);

	// Set executable bits
	for (const fm of config.files) {
		if (fm.executable) {
			const dst = path.join(
				outDir,
				fm.dst.replace("{TRIPLE}", config.tauriTriple),
			);
			if (fs.existsSync(dst)) {
				fs.chmodSync(dst, 0o755);
			}
		}
	}

	// Fix macOS dylib versioned install names (@rpath/libfoo.0.dylib → @rpath/libfoo.dylib)
	fixMacOsDylibNames(outDir, config);

	// Clean up archive
	if (fs.existsSync(archivePath)) {
		fs.unlinkSync(archivePath);
	}
}

async function main() {
	const args = process.argv.slice(2);
	let requestedTag: string | "latest" = PINNED_TAG;
	let platformFilter: string | null = null;

	for (let i = 0; i < args.length; i++) {
		if (args[i] === "--latest") {
			requestedTag = "latest";
		} else if (args[i] === "--tag" && args[i + 1]) {
			requestedTag = args[i + 1];
			i++;
		} else if (args[i] === "--platform" && args[i + 1]) {
			platformFilter = args[i + 1];
			i++;
		}
	}

	const tag = await resolveTag(requestedTag);
	console.log(`Updating llama.cpp binaries to ${tag}`);
	console.log(`Binaries directory: ${BINARIES_DIR}`);

	const platforms = platformFilter
		? { [platformFilter]: PLATFORMS[platformFilter] }
		: PLATFORMS;

	if (platformFilter && !PLATFORMS[platformFilter]) {
		console.error(
			`Unknown platform: ${platformFilter}. Available: ${Object.keys(PLATFORMS).join(", ")}`,
		);
		process.exit(1);
	}

	for (const [key, config] of Object.entries(platforms)) {
		await updatePlatform(key, config, tag);
	}

	console.log(`\nDone! Updated to ${tag}.`);
	console.log(
		"Remember to update PINNED_TAG in this script if you used --latest or --tag.",
	);
}

main().catch((err) => {
	console.error("Fatal:", err);
	process.exit(1);
});
