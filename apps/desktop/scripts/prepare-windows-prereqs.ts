import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const SRC_TAURI_DIR = path.resolve(SCRIPT_DIR, "../src-tauri");
const WORKSPACE_DIR = path.resolve(SRC_TAURI_DIR, "../../..");
const ORT_STAGER_MANIFEST = path.join(
	SCRIPT_DIR,
	"ort-runtime-stager/Cargo.toml",
);
const ORT_STAGER_LOCK = path.join(SCRIPT_DIR, "ort-runtime-stager/Cargo.lock");

const RUNTIME_DLLS = [
	"msvcp140.dll",
	"vcruntime140.dll",
	"vcruntime140_1.dll",
] as const;

const ARCHITECTURES = {
	x64: {
		redistArch: "x64",
		targetTriple: "x86_64-pc-windows-msvc",
		outputDir: path.join(SRC_TAURI_DIR, "binaries/win/x64"),
	},
	arm64: {
		redistArch: "arm64",
		targetTriple: "aarch64-pc-windows-msvc",
		outputDir: path.join(SRC_TAURI_DIR, "binaries/win/arm"),
	},
} as const;

type Architecture = keyof typeof ARCHITECTURES;

type CliOptions = {
	arch?: Architecture;
	force: boolean;
	redistDir?: string;
};

function parseArgs(): CliOptions {
	const options: CliOptions = {
		force: false,
	};

	for (let i = 2; i < process.argv.length; i++) {
		const arg = process.argv[i];

		if (arg === "--arch") {
			const arch = process.argv[++i] as Architecture | undefined;
			if (!arch || !(arch in ARCHITECTURES)) {
				throw new Error("--arch must be one of: x64, arm64");
			}
			options.arch = arch;
			continue;
		}

		if (arg === "--force") {
			options.force = true;
			continue;
		}

		if (arg === "--redist-dir") {
			const redistDir = process.argv[++i];
			if (!redistDir) {
				throw new Error("--redist-dir requires a path");
			}
			options.redistDir = path.resolve(redistDir);
			continue;
		}

		if (arg === "--help" || arg === "-h") {
			printHelp();
			process.exit(0);
		}

		throw new Error(`Unknown option: ${arg}`);
	}

	return options;
}

function selectedArchitectures(options: CliOptions): Architecture[] {
	if (!options.arch) return Object.keys(ARCHITECTURES) as Architecture[];

	return [options.arch];
}

function printHelp(): void {
	console.log(`Usage: bun run scripts/prepare-windows-prereqs.ts [--arch x64|arm64] [--redist-dir PATH] [--force]

Stages app-local Microsoft Visual C++ and ONNX Runtime/DirectML DLLs into:
  ${path.relative(process.cwd(), path.join(SRC_TAURI_DIR, "binaries/win"))}

The Windows Tauri configs bundle these files as resources so MSI, NSIS
and updater installs include the native runtime DLLs with the app.

By default this script locates the Visual Studio Redistributable directory
from VCToolsRedistDir, VCINSTALLDIR or vswhere. Set FLOWLIKE_VC_REDIST_DIR
or pass --redist-dir to use a specific redist folder. Existing staged DLLs
are kept unless --force is passed, so committed binaries stay reproducible.
`);
}

function unique(paths: string[]): string[] {
	const seen = new Set<string>();
	const result: string[] = [];

	for (const item of paths) {
		const key = process.platform === "win32" ? item.toLowerCase() : item;
		if (!seen.has(key)) {
			seen.add(key);
			result.push(item);
		}
	}

	return result;
}

function subdirectories(dir: string): string[] {
	try {
		return fs
			.readdirSync(dir, { withFileTypes: true })
			.filter((entry) => entry.isDirectory())
			.map((entry) => path.join(dir, entry.name));
	} catch {
		return [];
	}
}

function sortNewestFirst(paths: string[]): string[] {
	return [...paths].sort((a, b) =>
		path.basename(b).localeCompare(path.basename(a), undefined, {
			numeric: true,
			sensitivity: "base",
		}),
	);
}

function findCaseInsensitiveFile(dir: string, fileName: string): string | undefined {
	try {
		const target = fileName.toLowerCase();
		for (const entry of fs.readdirSync(dir)) {
			if (entry.toLowerCase() === target) {
				return path.join(dir, entry);
			}
		}
	} catch {
		return undefined;
	}
}

function hasRuntimeDlls(dir: string): boolean {
	return RUNTIME_DLLS.every((dll) => findCaseInsensitiveFile(dir, dll));
}

function fileHash(filePath: string): string {
	const hash = createHash("sha256");
	hash.update(fs.readFileSync(filePath));
	return hash.digest("hex");
}

function microsoftCrtDirs(archDir: string): string[] {
	return sortNewestFirst(
		subdirectories(archDir).filter((dir) =>
			/^Microsoft\.VC\d+\.CRT$/i.test(path.basename(dir)),
		),
	);
}

function candidateRuntimeDirs(root: string, arch: Architecture): string[] {
	const { redistArch } = ARCHITECTURES[arch];
	const candidates: string[] = [root];
	const archDir = path.join(root, redistArch);

	candidates.push(archDir);
	candidates.push(...microsoftCrtDirs(archDir));

	for (const versionDir of sortNewestFirst(subdirectories(root))) {
		const versionArchDir = path.join(versionDir, redistArch);
		candidates.push(versionArchDir);
		candidates.push(...microsoftCrtDirs(versionArchDir));
	}

	return candidates;
}

function detectVisualStudioRedistRoots(): string[] {
	if (process.platform !== "win32") {
		return [];
	}

	const roots: string[] = [];
	const vcToolsRedistDir = process.env.VCToolsRedistDir;
	const vcInstallDir = process.env.VCINSTALLDIR;

	if (vcToolsRedistDir) {
		roots.push(vcToolsRedistDir);
	}

	if (vcInstallDir) {
		roots.push(path.join(vcInstallDir, "Redist/MSVC"));
	}

	const vswhere = path.join(
		process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
		"Microsoft Visual Studio/Installer/vswhere.exe",
	);

	if (fs.existsSync(vswhere)) {
		try {
			const output = execFileSync(
				vswhere,
				[
					"-latest",
					"-products",
					"*",
					"-requires",
					"Microsoft.VisualStudio.Component.VC.Redist.14.Latest",
					"-property",
					"installationPath",
				],
				{ encoding: "utf8" },
			).trim();

			if (output) {
				roots.push(path.join(output, "VC/Redist/MSVC"));
			}
		} catch {
			// Fall back to common Visual Studio locations below.
		}
	}

	const programFiles = [
		process.env.ProgramFiles,
		process.env["ProgramFiles(x86)"],
		"C:\\Program Files",
		"C:\\Program Files (x86)",
	].filter((item): item is string => Boolean(item));

	for (const base of programFiles) {
		for (const edition of [
			"Enterprise",
			"Professional",
			"Community",
			"BuildTools",
		]) {
			roots.push(
				path.join(
					base,
					"Microsoft Visual Studio/2022",
					edition,
					"VC/Redist/MSVC",
				),
			);
		}
	}

	return unique(roots);
}

function resolveRuntimeDir(
	arch: Architecture,
	options: CliOptions,
): string | undefined {
	const roots = unique(
		[
			options.redistDir,
			process.env.FLOWLIKE_VC_REDIST_DIR,
			...detectVisualStudioRedistRoots(),
		].filter((item): item is string => Boolean(item)),
	);

	for (const root of roots) {
		for (const candidate of candidateRuntimeDirs(root, arch)) {
			if (hasRuntimeDlls(candidate)) {
				return candidate;
			}
		}
	}

	return undefined;
}

function stageRuntimeDlls(arch: Architecture, options: CliOptions): void {
	const runtimeDir = resolveRuntimeDir(arch, options);
	const config = ARCHITECTURES[arch];

	if (!runtimeDir) {
		throw new Error(
			[
				`Could not locate Visual C++ runtime DLLs for ${arch}.`,
				"Install the Visual Studio C++ build tools with the latest v14 redistributable component,",
				"or set FLOWLIKE_VC_REDIST_DIR / --redist-dir to a folder containing Microsoft.VC143.CRT.",
			].join(" "),
		);
	}

	fs.mkdirSync(config.outputDir, { recursive: true });

	for (const dll of RUNTIME_DLLS) {
		const source = findCaseInsensitiveFile(runtimeDir, dll);
		if (!source) {
			throw new Error(`Missing ${dll} in ${runtimeDir}`);
		}

		const dest = path.join(config.outputDir, `${dll}-${config.targetTriple}`);

		if (!options.force && fs.existsSync(dest)) {
			if (fileHash(source) === fileHash(dest)) {
				console.log(`Already staged: ${path.relative(process.cwd(), dest)}`);
			} else {
				console.log(
					`Keeping staged ${path.relative(process.cwd(), dest)}; use --force to refresh from ${runtimeDir}`,
				);
			}
			continue;
		}

		fs.copyFileSync(source, dest);
		console.log(
			`Staged ${path.basename(source)} -> ${path.relative(process.cwd(), dest)}`,
		);
	}
}

function cargoTargetDir(): string {
	const configured = process.env.CARGO_TARGET_DIR;
	const base = configured
		? path.resolve(WORKSPACE_DIR, configured)
		: path.join(WORKSPACE_DIR, "target");
	// ort-sys intentionally avoids overwriting an existing copied DLL. Isolating the
	// helper by its lockfile hash guarantees an ORT dependency update cannot reuse a
	// DirectML.dll left behind by the previous runtime version.
	const runtimeKey = fileHash(ORT_STAGER_LOCK).slice(0, 16);
	return path.join(base, "ort-runtime-stager", runtimeKey);
}

function stageDirectMlDll(arch: Architecture, options: CliOptions): void {
	const config = ARCHITECTURES[arch];
	execFileSync(
		"cargo",
		[
			"check",
			"--manifest-path",
			ORT_STAGER_MANIFEST,
			"--target",
			config.targetTriple,
			"--target-dir",
			cargoTargetDir(),
			"--locked",
		],
		{
			cwd: WORKSPACE_DIR,
			stdio: "inherit",
		},
	);

	const source = path.join(
		cargoTargetDir(),
		config.targetTriple,
		"debug",
		"DirectML.dll",
	);
	if (!fs.existsSync(source)) {
		throw new Error(
			`ort-sys did not stage DirectML.dll at the expected path: ${source}`,
		);
	}

	fs.mkdirSync(config.outputDir, { recursive: true });
	const destination = path.join(
		config.outputDir,
		`DirectML.dll-${config.targetTriple}`,
	);
	if (!options.force && fs.existsSync(destination)) {
		if (fileHash(source) === fileHash(destination)) {
			console.log(`Already staged: ${path.relative(process.cwd(), destination)}`);
			return;
		}
		console.log(
			`Refreshing ${path.relative(process.cwd(), destination)} from ort-sys`,
		);
	}

	fs.copyFileSync(source, destination);
	console.log(
		`Staged DirectML.dll -> ${path.relative(process.cwd(), destination)}`,
	);
}

async function main(): Promise<void> {
	const options = parseArgs();

	for (const arch of selectedArchitectures(options)) {
		stageRuntimeDlls(arch, options);
		stageDirectMlDll(arch, options);
	}
}

await main();
