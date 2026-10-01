// tools/make-licenses.ts
// Usage: bun tools/make-licenses.ts
// Creates ./thirdparty/* and a merged ./THIRD-PARTY-NOTICES.txt
// Hand-written sections in ./thirdparty/manual-notices/*.md are kept and prepended.

import { constants as FS } from "node:fs";
import {
	access,
	mkdir,
	readFile,
	readdir,
	rm,
	writeFile,
} from "node:fs/promises";
import { dirname, resolve } from "node:path";

const ROOT = resolve(".");
const OUT_DIR = resolve("./thirdparty");
const MANUAL_NOTICES_DIR = resolve(OUT_DIR, "manual-notices");
const FINAL = resolve(
	"./apps/desktop/src-tauri/assets/THIRD-PARTY-NOTICES.txt",
);

// Rust outputs
const RUST_JSON = resolve(OUT_DIR, "cargo-licenses.json");
const RUST_MD = resolve(OUT_DIR, "THIRD-PARTY-NOTICES.rust.md");

// JS/TS inputs & outputs
const WEB_TARGETS = [
	{
		label: "./package.json",
		pkgJson: "./package.json",
		out: resolve(OUT_DIR, "web-root.txt"),
	},
	{
		label: "./apps/desktop/package.json",
		pkgJson: "./apps/desktop/package.json",
		out: resolve(OUT_DIR, "web-desktop.txt"),
	},
	{
		label: "./packages/ui/package.json",
		pkgJson: "./packages/ui/package.json",
		out: resolve(OUT_DIR, "web-ui.txt"),
	},
];

// Potential Cargo manifests (workspace root and tauri crate)
const CARGO_MANIFESTS = ["./Cargo.toml", "./apps/desktop/src-tauri/Cargo.toml"];

type RustLicenseRow = {
	name: string;
	version: string;
	license?: string;
	repository?: string;
	description?: string;
	license_text?: string;
	licenseText?: string;
};

async function exists(p: string) {
	try {
		await access(p, FS.F_OK);
		return true;
	} catch {
		return false;
	}
}

async function run(cmd: string[], cwd = ROOT) {
	const proc = Bun.spawn({
		cmd,
		cwd,
		stdout: "pipe",
		stderr: "pipe",
	});
	const outP = new Response(proc.stdout).text();
	const errP = new Response(proc.stderr).text();
	const code = await proc.exited;
	const stdout = await outP;
	const stderr = await errP;
	if (code !== 0) {
		const pretty = `Command failed (${code}): ${cmd.join(" ")}\n${stderr || stdout}`;
		throw new Error(pretty);
	}
	return { stdout, stderr, code };
}

const GENERATED_OUTPUTS = [
	RUST_JSON,
	RUST_MD,
	...WEB_TARGETS.map((target) => target.out),
];

async function ensureOutDir() {
	await mkdir(OUT_DIR, { recursive: true });
	await Promise.all(GENERATED_OUTPUTS.map((path) => rm(path, { force: true })));
}

async function ensureCargoLicense() {
	try {
		await run(["cargo", "license", "--version"]);
	} catch {
		console.log("🔧 Installing cargo-license …");
		await run(["cargo", "install", "--locked", "cargo-license"]);
	}
}

async function fileContains(path: string, needle: string) {
	try {
		const s = await readFile(path, "utf8");
		return s.includes(needle);
	} catch {
		return false;
	}
}

function renderRustMarkdown(rows: RustLicenseRow[]) {
	const header = `# Third-Party Notices — Rust (Cargo)
This section lists Rust crates used by this application, with license identifiers and full license texts when available.

`;
	const body = rows
		.map((r) => {
			const lic = r.license ?? "UNKNOWN";
			const repo = r.repository ? `Repository: ${r.repository}\n` : "";
			const desc = r.description ? `${r.description}\n` : "";
			const text = (r.license_text ?? r.licenseText ?? "").trim();
			const textBlock = text
				? `\n----- LICENSE TEXT (${lic}) -----\n${text}\n`
				: "";
			return `## ${r.name} ${r.version} — ${lic}\n${repo}${desc}${textBlock}`;
		})
		.join("\n\n");
	return `${header}${body}\n`;
}

function dedupeRust(rows: RustLicenseRow[]): RustLicenseRow[] {
	const seen = new Map<string, RustLicenseRow>();
	for (const r of rows) {
		const key = `${r.name}@${r.version}::${r.license ?? ""}`;
		if (!seen.has(key)) seen.set(key, r);
	}
	return [...seen.values()].sort((a, b) =>
		(a.name + a.version).localeCompare(b.name + b.version),
	);
}

async function gatherRust() {
	const manifests: string[] = [];
	for (const m of CARGO_MANIFESTS) {
		if (await exists(m)) manifests.push(m);
	}
	if (manifests.length === 0) {
		console.log("ℹ️  No Cargo.toml found; skipping Rust license collection.");
		return;
	}

	await ensureCargoLicense();

	const allRows: RustLicenseRow[] = [];
	for (const manifest of manifests) {
		const args = [
			"license",
			"--json",
			"--avoid-build-deps",
			"--avoid-dev-deps",
		];

		console.log(`🦀 cargo ${args.join(" ")}`);
		const { stdout } = await run(["cargo", ...args]);
		try {
			const rows: RustLicenseRow[] = JSON.parse(stdout);
			allRows.push(...rows);
		} catch (e) {
			throw new Error(
				`Failed to parse cargo-license JSON for ${manifest}: ${(e as Error).message}`,
			);
		}
	}

	const deduped = dedupeRust(allRows);
	await writeFile(RUST_JSON, JSON.stringify(deduped, null, 2), "utf8");
	await writeFile(RUST_MD, renderRustMarkdown(deduped), "utf8");
}

async function gatherWeb() {
	for (const target of WEB_TARGETS) {
		const pkgJsonAbs = resolve(target.pkgJson);
		if (!(await exists(pkgJsonAbs))) {
			console.log(`ℹ️  ${target.label} not found; skipping.`);
			continue;
		}

		// Try generate-license-file first (includes full license texts when it can)
		try {
			console.log(`📦 bunx generate-license-file for ${target.label}`);
			await run(
				[
					"bunx",
					"generate-license-file",
					"--input",
					pkgJsonAbs,
					"--output",
					target.out,
					"--overwrite",
				],
				dirname(pkgJsonAbs),
			);
			continue; // success
		} catch (e) {
			console.warn(
				`⚠️  generate-license-file failed for ${target.label}: ${(e as Error).message}`,
			);
		}

		// Fallback: license-checker-rseidelsohn (summary/markdown)
		try {
			console.log(`📦 bunx license-checker-rseidelsohn for ${target.label}`);
			const startDir = dirname(pkgJsonAbs);
			const { stdout } = await run(
				[
					"bunx",
					"license-checker-rseidelsohn",
					"--production",
					"--summary",
					"--markdown",
					"--relativeLicensePath",
					"--start",
					startDir,
				],
				startDir,
			);
			const header = `# Third-Party Notices — ${target.label}\n\n`;
			await writeFile(target.out, header + stdout, "utf8");
		} catch (e) {
			console.error(
				`❌ Could not generate JS/TS licenses for ${target.label}: ${(e as Error).message}`,
			);
		}
	}
}

async function readManualNotices() {
	if (!(await exists(MANUAL_NOTICES_DIR))) return "";
	const names = (await readdir(MANUAL_NOTICES_DIR))
		.filter((name) => name.endsWith(".md"))
		.sort();
	const notices = await Promise.all(
		names.map((name) => readFile(resolve(MANUAL_NOTICES_DIR, name), "utf8")),
	);
	return notices.join("");
}

export async function mergeAll(finalPath = FINAL) {
	let out = `# THIRD-PARTY NOTICES

_This file aggregates licenses for third-party software used by this application._

`;

	out += await readManualNotices();

	if (await exists(RUST_MD)) {
		out += await readFile(RUST_MD, "utf8");
		out += "\n\n";
	}

	for (const t of WEB_TARGETS) {
		if (await exists(t.out)) {
			out += `# Third-Party Notices — ${t.label}\n\n`;
			out += await readFile(t.out, "utf8");
			out += "\n\n";
		}
	}

	await writeFile(finalPath, out, "utf8");
}

async function main() {
	console.log("▶️  Preparing ./thirdparty …");
	await ensureOutDir();

	console.log("🦀 Collecting Rust licenses …");
	await gatherRust();

	console.log("🕸️  Collecting JS/TS licenses …");
	await gatherWeb();

	console.log("🧩 Merging into THIRD-PARTY-NOTICES.txt …");
	await mergeAll();

	console.log("✅ Done.");
	console.log(`• Intermediates: ${OUT_DIR}`);
	console.log(`• Final:        ${FINAL}`);
}

if (import.meta.main) {
	main().catch((err) => {
		console.error(
			"❌ License export failed:\n",
			err?.stack || err?.message || String(err),
		);
		// Bun provides process.exit
		process.exit(1);
	});
}
