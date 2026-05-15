import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dir, "..");

function readProjectFile(relativePath: string) {
	return readFileSync(path.join(root, relativePath), "utf8");
}

function parseNpmrc(source: string) {
	const config = new Map<string, string>();

	for (const line of source.split(/\r?\n/)) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith(";")) continue;

		const equalsIndex = trimmed.indexOf("=");
		if (equalsIndex === -1) continue;

		config.set(trimmed.slice(0, equalsIndex), trimmed.slice(equalsIndex + 1));
	}

	return config;
}

function expectTomlScalar(source: string, key: string, value: string) {
	expect(source).toMatch(new RegExp(`^\\s*${key}\\s*=\\s*${value}\\s*$`, "m"));
}

function expectYamlScalar(source: string, key: string, value: string) {
	expect(source).toMatch(new RegExp(`^${key}:\\s*${value}\\s*$`, "m"));
}

describe("package manager supply-chain policy", () => {
	test("npm keeps the project policy pinned", () => {
		const npmrc = parseNpmrc(readProjectFile(".npmrc"));

		expect(npmrc.get("min-release-age")).toBe("3");
		expect(npmrc.get("save-exact")).toBe("true");
		expect(npmrc.get("strict-ssl")).toBe("true");
		expect(npmrc.get("audit-level")).toBe("high");
		expect(npmrc.get("allow-directory")).toBe("root");
		expect(npmrc.get("allow-file")).toBe("root");
		expect(npmrc.get("allow-git")).toBe("root");
		expect(npmrc.get("allow-remote")).toBe("root");
	});

	test("bun keeps the age gate and exact-save policy enabled", () => {
		const bunfig = readProjectFile("bunfig.toml");

		expectTomlScalar(bunfig, "linker", '"hoisted"');
		expectTomlScalar(bunfig, "exact", "true");
		expectTomlScalar(bunfig, "minimumReleaseAge", "259200");
		expect(bunfig).toContain('"@flow-like/sdk"');
		expect(bunfig).toContain('"flow-like-web"');
	});

	test("pnpm keeps the age gate, exact-save policy, and exotic subdep block enabled", () => {
		const pnpmWorkspace = readProjectFile("pnpm-workspace.yaml");

		expectYamlScalar(pnpmWorkspace, "minimumReleaseAge", "4320");
		expectYamlScalar(pnpmWorkspace, "savePrefix", '""');
		expectYamlScalar(pnpmWorkspace, "blockExoticSubdeps", "true");
		expectYamlScalar(pnpmWorkspace, "strictDepBuilds", "true");
		expect(pnpmWorkspace).toContain('- "@flow-like/*"');
	});

	test("install-script builder allowlists stay narrow", () => {
		const packageJson = JSON.parse(readProjectFile("package.json"));
		const pnpmWorkspace = readProjectFile("pnpm-workspace.yaml");

		expect(packageJson.trustedDependencies).toEqual(["esbuild", "fsevents", "sharp"]);
		expect(pnpmWorkspace).toMatch(/^  core-js: false$/m);
		expect(pnpmWorkspace).toMatch(/^  esbuild: true$/m);
		expect(pnpmWorkspace).toMatch(/^  fsevents: true$/m);
		expect(pnpmWorkspace).toMatch(/^  sharp: true$/m);
	});

	test("bun accepts the committed lockfile under the hardened config", () => {
		const tempRoot = path.join(root, "tmp", "package-manager-security");
		mkdirSync(tempRoot, { recursive: true });
		const tempDir = mkdtempSync(path.join(tempRoot, "bun-"));

		const result = spawnSync("bun", ["install", "--dry-run", "--frozen-lockfile"], {
			cwd: root,
			encoding: "utf8",
			env: {
				...process.env,
				TMPDIR: tempDir,
			},
		});

		rmSync(tempDir, { force: true, recursive: true });

		expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
	});
});
