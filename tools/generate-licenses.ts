// tools/generate-licenses.ts
// Comprehensive license generation with full license texts
// Usage: bun tools/generate-licenses.ts

import { constants as FS, existsSync, readdirSync } from "node:fs";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const OUT_DIR = resolve(ROOT, "thirdparty");

// Output files
const RUST_LICENSES_JSON = resolve(OUT_DIR, "cargo-licenses.json");
const NPM_LICENSES_JSON = resolve(OUT_DIR, "npm-licenses.json");
const COMBINED_LICENSES = resolve(OUT_DIR, "all-licenses.json");

// Cargo registry path
const CARGO_REGISTRY = join(homedir(), ".cargo", "registry", "src");

// All package.json files to scan (excluding build artifacts and node_modules)
const NPM_TARGETS = [
	"./package.json",
	"./apps/desktop/package.json",
	"./apps/web/package.json",
	"./apps/website/package.json",
	"./apps/docs/package.json",
	"./apps/embedded/package.json",
	"./packages/ui/package.json",
	"./packages/api/package.json",
	"./apps/backend/local/runtime/package.json",
	"./apps/backend/local/api/package.json",
];

interface RustLicense {
	name: string;
	version: string;
	authors: string | null;
	repository: string | null;
	license: string | null;
	description: string | null;
	license_text?: string;
}

interface NpmLicenseEntry {
	licenses: string;
	repository?: string;
	publisher?: string;
	email?: string;
	path?: string;
	licenseFile?: string;
	licenseText?: string;
}

interface CombinedLicense {
	name: string;
	version: string;
	authors: string | null;
	repository: string | null;
	license: string | null;
	description: string | null;
	source: "rust" | "npm";
	licenseText?: string;
}

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
		console.warn(
			`Command warning (${code}): ${cmd.join(" ")}\n${stderr || stdout}`,
		);
	}
	return { stdout, stderr, code };
}

async function ensureCargoLicense() {
	const { code } = await run(["cargo", "license", "--help"]);
	if (code !== 0) {
		console.log("🔧 Installing cargo-license …");
		await run(["cargo", "install", "--locked", "cargo-license"]);
	}
}

// Find license file in a directory
const LICENSE_FILE_PATTERNS = [
	"LICENSE",
	"LICENSE.txt",
	"LICENSE.md",
	"LICENSE-MIT",
	"LICENSE-APACHE",
	"COPYING",
	"license",
	"License",
];

async function findLicenseText(
	crateName: string,
	version: string,
): Promise<string | undefined> {
	if (!existsSync(CARGO_REGISTRY)) return undefined;

	// Find the registry index directory (e.g., github.com-1ecc6299db9ec823)
	const indexDirs = readdirSync(CARGO_REGISTRY);

	for (const indexDir of indexDirs) {
		const cratePath = join(CARGO_REGISTRY, indexDir, `${crateName}-${version}`);
		if (existsSync(cratePath)) {
			// Look for license files
			for (const pattern of LICENSE_FILE_PATTERNS) {
				const licensePath = join(cratePath, pattern);
				if (existsSync(licensePath)) {
					try {
						return await readFile(licensePath, "utf8");
					} catch {
						// Continue to next pattern
					}
				}
			}
		}
	}

	return undefined;
}

async function gatherRustLicenses(): Promise<RustLicense[]> {
	console.log("🦀 Collecting Rust licenses with cargo-license...");
	await ensureCargoLicense();

	const { stdout } = await run([
		"cargo",
		"license",
		"--json",
		"--avoid-build-deps",
		"--avoid-dev-deps",
		"-a", // include authors
	]);

	let licenses: RustLicense[] = [];
	try {
		licenses = JSON.parse(stdout);
	} catch (e) {
		console.error("Failed to parse cargo-license output:", e);
		return [];
	}

	// Dedupe by name@version
	const seen = new Map<string, RustLicense>();
	for (const lic of licenses) {
		const key = `${lic.name}@${lic.version}`;
		if (!seen.has(key)) {
			seen.set(key, lic);
		}
	}

	const deduped = [...seen.values()].sort((a, b) =>
		a.name.localeCompare(b.name),
	);

	// Try to find license texts from cargo cache
	console.log("📄 Reading Rust license texts from cargo cache...");
	let foundCount = 0;
	for (const lic of deduped) {
		const text = await findLicenseText(lic.name, lic.version);
		if (text) {
			lic.license_text = text;
			foundCount++;
		}
	}
	console.log(
		`   Found license text for ${foundCount}/${deduped.length} crates`,
	);

	return deduped;
}

async function gatherNpmLicenses(): Promise<Record<string, NpmLicenseEntry>> {
	console.log("📦 Collecting npm licenses from all package.json files...");

	const allLicenses: Record<string, NpmLicenseEntry> = {};

	for (const target of NPM_TARGETS) {
		const pkgPath = resolve(ROOT, target);
		if (!(await exists(pkgPath))) {
			console.log(`  ⏭️  Skipping ${target} (not found)`);
			continue;
		}

		const dir = dirname(pkgPath);
		const nodeModules = resolve(dir, "node_modules");

		if (!(await exists(nodeModules))) {
			console.log(`  ⏭️  Skipping ${target} (no node_modules)`);
			continue;
		}

		console.log(`  📦 Scanning ${target}...`);

		try {
			const { stdout, code } = await run(
				[
					"bunx",
					"license-checker-rseidelsohn",
					"--json",
					"--production",
					"--start",
					dir,
				],
				dir,
			);

			if (code === 0) {
				const licenses = JSON.parse(stdout) as Record<string, NpmLicenseEntry>;
				// Merge, preferring entries with more info
				for (const [key, value] of Object.entries(licenses)) {
					if (
						!allLicenses[key] ||
						(value.licenseFile && !allLicenses[key].licenseFile)
					) {
						allLicenses[key] = value;
					}
				}
			}
		} catch (e) {
			console.warn(`  ⚠️  Failed to scan ${target}:`, e);
		}
	}

	return allLicenses;
}

async function readNpmLicenseTexts(
	npmLicenses: Record<string, NpmLicenseEntry>,
): Promise<Record<string, NpmLicenseEntry>> {
	console.log("📄 Reading npm license text files...");

	const result: Record<string, NpmLicenseEntry> = {};
	let foundCount = 0;

	for (const [key, entry] of Object.entries(npmLicenses)) {
		const newEntry = { ...entry };

		if (entry.licenseFile && existsSync(entry.licenseFile)) {
			try {
				const text = await readFile(entry.licenseFile, "utf8");
				// Only include if it looks like a license text
				const lower = text.toLowerCase();
				if (
					lower.includes("license") ||
					lower.includes("permission") ||
					lower.includes("copyright") ||
					lower.includes("mit") ||
					lower.includes("apache") ||
					lower.includes("bsd")
				) {
					newEntry.licenseText = text;
					foundCount++;
				}
			} catch {
				// Skip if can't read
			}
		}

		result[key] = newEntry;
	}

	const total = Object.keys(result).length;
	console.log(`   Found license text for ${foundCount}/${total} packages`);

	return result;
}

function combineAllLicenses(
	rustLicenses: RustLicense[],
	npmLicenses: Record<string, NpmLicenseEntry>,
): CombinedLicense[] {
	const combined: CombinedLicense[] = [];

	// Add Rust licenses
	for (const lic of rustLicenses) {
		combined.push({
			name: lic.name,
			version: lic.version,
			authors: lic.authors,
			repository: lic.repository,
			license: lic.license,
			description: lic.description,
			source: "rust",
			licenseText: lic.license_text,
		});
	}

	// Add npm licenses
	for (const [key, entry] of Object.entries(npmLicenses)) {
		// Skip internal packages
		if (key.includes("flow-like") || key.includes("@flow-like")) continue;

		const match = key.match(/^(.+)@(\d+\.\d+\.\d+.*)$/);
		const name = match ? match[1] : key;
		const version = match ? match[2] : "unknown";

		combined.push({
			name,
			version,
			authors: entry.publisher || null,
			repository: entry.repository || null,
			license: entry.licenses || null,
			description: null,
			source: "npm",
			licenseText: entry.licenseText,
		});
	}

	// Dedupe by name@version@source
	const seen = new Map<string, CombinedLicense>();
	for (const lic of combined) {
		const key = `${lic.name}@${lic.version}@${lic.source}`;
		if (!seen.has(key)) {
			seen.set(key, lic);
		}
	}

	return [...seen.values()].sort((a, b) => {
		const nameCompare = a.name.localeCompare(b.name);
		if (nameCompare !== 0) return nameCompare;
		return a.source.localeCompare(b.source);
	});
}

async function main() {
	console.log("▶️  Starting comprehensive license generation...\n");

	// Ensure output directory exists
	await mkdir(OUT_DIR, { recursive: true });

	// Gather Rust licenses
	const rustLicenses = await gatherRustLicenses();
	console.log(`   Total: ${rustLicenses.length} Rust crates\n`);

	// Write Rust output
	await writeFile(
		RUST_LICENSES_JSON,
		JSON.stringify(rustLicenses, null, 2),
		"utf8",
	);
	console.log(`   Written to ${relative(ROOT, RUST_LICENSES_JSON)}\n`);

	// Gather npm licenses
	const npmLicenses = await gatherNpmLicenses();
	const npmCount = Object.keys(npmLicenses).length;
	console.log(`   Total: ${npmCount} npm packages\n`);

	// Read license texts
	const npmWithTexts = await readNpmLicenseTexts(npmLicenses);

	// Write npm output
	await writeFile(
		NPM_LICENSES_JSON,
		JSON.stringify(npmWithTexts, null, 2),
		"utf8",
	);
	console.log(`   Written to ${relative(ROOT, NPM_LICENSES_JSON)}\n`);

	// Combine all licenses
	const combined = combineAllLicenses(rustLicenses, npmWithTexts);
	await writeFile(COMBINED_LICENSES, JSON.stringify(combined, null, 2), "utf8");
	console.log(`   Combined ${combined.length} total licenses`);
	console.log(`   Written to ${relative(ROOT, COMBINED_LICENSES)}\n`);

	// Stats
	const withText = combined.filter((l) => l.licenseText).length;
	console.log("📊 Summary:");
	console.log(`   Total packages: ${combined.length}`);
	console.log(`   Rust crates: ${rustLicenses.length}`);
	console.log(`   npm packages: ${npmCount}`);
	console.log(
		`   With license text: ${withText} (${Math.round((withText / combined.length) * 100)}%)`,
	);

	console.log("\n✅ Done!");
}

main().catch((err) => {
	console.error("❌ License generation failed:", err);
	process.exit(1);
});
