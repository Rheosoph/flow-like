/** Build the unixODBC driver manager that macOS desktop builds link and bundle. */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const VERSION = "2.3.14";
const SHA256 =
	"4e2814de3e01fc30b0b9f75e83bb5aba91ab0384ee951286504bb70205524771";
const SOURCE_URL = `https://www.unixodbc.org/unixODBC-${VERSION}.tar.gz`;
const MACOS_TARGET = "14.0";
// Driver installers on macOS register in iODBC's location; ODBCSYSINI overrides it.
const SYSCONFDIR = "/Library/ODBC";
const LIBRARY = "libodbc.2.dylib";
// `-lodbc` resolves libodbc.dylib; without it the linker falls through to Homebrew's copy.
const LINK_NAME = "libodbc.dylib";
const ROOT = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../src-tauri/binaries/odbc",
);
const ARCHES = {
	"aarch64-apple-darwin": "arm64",
	"x86_64-apple-darwin": "x86_64",
} as const;
type Target = keyof typeof ARCHES;

function isTarget(value: string): value is Target {
	return value in ARCHES;
}

function run(
	command: string,
	args: string[],
	cwd: string,
	env: Partial<NodeJS.ProcessEnv> = {},
) {
	execFileSync(command, args, {
		cwd,
		env: { ...process.env, ...env },
		stdio: ["ignore", "ignore", "inherit"],
	});
}

function output(command: string, args: string[]) {
	return execFileSync(command, args, { encoding: "utf8" });
}

async function sourceArchive(): Promise<string> {
	const archive = path.join(ROOT, `.unixODBC-${VERSION}.tar.gz`);
	if (!fs.existsSync(archive)) {
		const response = await fetch(SOURCE_URL);
		if (!response.ok)
			throw new Error(
				`Downloading ${SOURCE_URL} failed: HTTP ${response.status}`,
			);
		fs.mkdirSync(ROOT, { recursive: true });
		fs.writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
	}
	const digest = createHash("sha256")
		.update(fs.readFileSync(archive))
		.digest("hex");
	if (digest !== SHA256) {
		fs.rmSync(archive);
		throw new Error(
			`unixODBC ${VERSION} archive checksum mismatch: got ${digest}`,
		);
	}
	return archive;
}

// The bundle may only carry a library that loads from Contents/Frameworks on a stock Mac.
function verify(library: string, arch: string) {
	const archs = output("lipo", ["-archs", library]).trim();
	if (archs !== arch)
		throw new Error(`${library} is built for ${archs}, expected ${arch}`);
	const dependencies = output("otool", ["-L", library])
		.split("\n")
		.slice(1)
		.map((line) => line.trim().split(" ")[0])
		.filter(Boolean);
	const foreign = dependencies.filter(
		(dependency) =>
			dependency !== `@rpath/${LIBRARY}` && !dependency.startsWith("/usr/lib/"),
	);
	if (foreign.length > 0)
		throw new Error(
			`${library} depends on libraries the app does not ship: ${foreign.join(", ")}`,
		);
}

export async function prepare(target: Target, force = false) {
	const destination = path.join(ROOT, target);
	const marker = path.join(destination, "VERSION");
	if (
		!force &&
		fs.existsSync(path.join(destination, LIBRARY)) &&
		fs.existsSync(path.join(destination, LINK_NAME)) &&
		fs.existsSync(marker) &&
		fs.readFileSync(marker, "utf8").trim() === VERSION
	)
		return destination;
	if (process.platform !== "darwin")
		throw new Error("Build the macOS unixODBC library on macOS.");

	const arch = ARCHES[target];
	const work = fs.mkdtempSync(path.join(os.tmpdir(), "flow-like-unixodbc-"));
	try {
		run("tar", ["-xzf", await sourceArchive(), "-C", work], work);
		const source = path.join(work, `unixODBC-${VERSION}`);
		// The bundled libltdl is folded into libodbc, so the app ships a single library.
		// `make install` is skipped: it would write odbcinst.ini into SYSCONFDIR.
		run(
			path.join(source, "configure"),
			[
				`--host=${target}`,
				`--prefix=${path.join(work, "prefix")}`,
				`--sysconfdir=${SYSCONFDIR}`,
				"--with-included-ltdl",
				"--disable-dependency-tracking",
			],
			source,
			{
				CC: `clang -arch ${arch}`,
				CFLAGS: `-O2 -mmacosx-version-min=${MACOS_TARGET}`,
			},
		);
		run("make", [`-j${os.availableParallelism()}`], source);

		const staging = path.join(work, "staging");
		fs.mkdirSync(staging);
		const library = path.join(staging, LIBRARY);
		fs.copyFileSync(path.join(source, "DriverManager/.libs", LIBRARY), library);
		fs.chmodSync(library, 0o755);
		run("install_name_tool", ["-id", `@rpath/${LIBRARY}`, library], staging);
		verify(library, arch);
		fs.copyFileSync(
			path.join(source, "COPYING"),
			path.join(staging, "COPYING"),
		);
		fs.symlinkSync(LIBRARY, path.join(staging, LINK_NAME));
		fs.writeFileSync(path.join(staging, "VERSION"), `${VERSION}\n`);

		fs.rmSync(destination, { recursive: true, force: true });
		fs.mkdirSync(ROOT, { recursive: true });
		fs.cpSync(staging, destination, {
			recursive: true,
			verbatimSymlinks: true,
		});
		console.log(`Prepared unixODBC ${VERSION} for ${target}`);
		return destination;
	} finally {
		fs.rmSync(work, { recursive: true, force: true });
	}
}

if (import.meta.main) {
	const args = process.argv.slice(2);
	if (args.includes("--help")) {
		console.log(
			"bun scripts/prepare-odbc.ts [--target aarch64-apple-darwin|x86_64-apple-darwin] [--force]",
		);
	} else {
		const index = args.indexOf("--target");
		const target =
			index >= 0
				? args[index + 1]
				: process.arch === "arm64"
					? "aarch64-apple-darwin"
					: "x86_64-apple-darwin";
		if (!target || !isTarget(target))
			throw new Error(
				`--target must be one of ${Object.keys(ARCHES).join(", ")}`,
			);
		await prepare(target, args.includes("--force"));
	}
}
