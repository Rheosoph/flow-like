import { afterAll, beforeAll, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import {
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { verifyUpdaterSignature } from "../verify-updater-signature";

const require = createRequire(import.meta.url);
const repository = join(import.meta.dir, "../../..");
const cli =
	process.env.TAURI_CLI_JS ??
	join(dirname(require.resolve("@tauri-apps/cli/package.json")), "tauri.js");
const temporary = mkdtempSync(join(tmpdir(), "flow-like-updater-signing-"));
const payload = Buffer.from("Temporary updater signing compatibility probe\n");
const payloadPath = join(temporary, "payload");
const version = "1.2.3";
const fixtures: { publicKey: string; signature: string }[] = [];

function runCli(args: string[], env: NodeJS.ProcessEnv = {}) {
	const environment = { ...process.env };
	for (const name of [
		"TAURI_SIGNING_PRIVATE_KEY",
		"TAURI_SIGNING_PRIVATE_KEY_PATH",
		"TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
	]) {
		delete environment[name];
	}
	return execFileSync(process.execPath, [cli, ...args], {
		env: { ...environment, ...env },
		stdio: ["ignore", "pipe", "pipe"],
		encoding: "utf8",
	});
}

beforeAll(() => {
	writeFileSync(payloadPath, payload);
	for (const [index, password] of ["", "throwaway-test-password"].entries()) {
		const keyPath = join(temporary, `key-${index}`);
		runCli([
			"signer",
			"generate",
			"--ci",
			"--password",
			password,
			"--write-keys",
			keyPath,
		]);
		runCli(["signer", "sign", "--app-version", version, payloadPath], {
			TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, "utf8"),
			TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password,
		});
		fixtures.push({
			publicKey: readFileSync(`${keyPath}.pub`, "utf8"),
			signature: readFileSync(`${payloadPath}.sig`, "utf8"),
		});
	}
}, 30_000);
afterAll(() => rmSync(temporary, { recursive: true, force: true }));

test("CI installs the same signing CLI as the desktop dependency", () => {
	const tools = JSON.parse(
		readFileSync(
			join(repository, ".github/release-tools/package.json"),
			"utf8",
		),
	);
	const desktop = JSON.parse(
		readFileSync(join(repository, "apps/desktop/package.json"), "utf8"),
	);
	const installed = JSON.parse(
		readFileSync(join(dirname(cli), "package.json"), "utf8"),
	);
	expect(installed.version).toBe(tools.dependencies["@tauri-apps/cli"]);
	expect(desktop.devDependencies["@tauri-apps/cli"].replace(/^[~^]/, "")).toBe(
		installed.version,
	);
	expect(runCli(["--version"]).trim()).toBe(`tauri-cli ${installed.version}`);
	// Native CI has no monorepo node_modules; its package script uses the tools on PATH.
	const scriptDirectory = join(temporary, "desktop");
	mkdirSync(scriptDirectory);
	writeFileSync(
		join(scriptDirectory, "package.json"),
		JSON.stringify({ scripts: { tauri: desktop.scripts.tauri } }),
	);
	const scriptVersion = execFileSync(
		process.execPath,
		["run", "tauri", "--version"],
		{
			cwd: scriptDirectory,
			env: {
				...process.env,
				PATH: `${join(dirname(cli), "../../.bin")}${delimiter}${process.env.PATH ?? ""}`,
			},
			stdio: ["ignore", "pipe", "pipe"],
			encoding: "utf8",
		},
	).trim();
	expect(scriptVersion).toBe(`tauri-cli ${installed.version}`);
});

test.each([0, 1])(
	"signer binds the version with password fixture %i",
	(index) => {
		const { signature, publicKey } = fixtures[index];
		expect(() =>
			verifyUpdaterSignature(signature, publicKey, version, payload),
		).not.toThrow();
	},
);

test("rejects a manifest version that relabels an older artifact", () => {
	expect(() =>
		verifyUpdaterSignature(
			fixtures[0].signature,
			fixtures[0].publicKey,
			"2.0.0",
		),
	).toThrow("signed version");
});

test("rejects a wrong release public key", () => {
	expect(() =>
		verifyUpdaterSignature(
			fixtures[0].signature,
			fixtures[1].publicKey,
			version,
		),
	).toThrow("public key");
});

test("rejects a forged trusted comment", () => {
	const forged = Buffer.from(
		Buffer.from(fixtures[0].signature, "base64")
			.toString()
			.replace("version:1.2.3", "version:2.0.0"),
	).toString("base64");
	expect(() =>
		verifyUpdaterSignature(forged, fixtures[0].publicKey, "2.0.0"),
	).toThrow("authentication failed");
});

test("rejects changed payload bytes", () => {
	expect(() =>
		verifyUpdaterSignature(
			fixtures[0].signature,
			fixtures[0].publicKey,
			version,
			Buffer.from("changed"),
		),
	).toThrow("payload signature");
});

test("rejects a legacy signature without version binding", () => {
	runCli(["signer", "sign", payloadPath], {
		TAURI_SIGNING_PRIVATE_KEY: readFileSync(join(temporary, "key-0"), "utf8"),
	});
	expect(() =>
		verifyUpdaterSignature(
			readFileSync(`${payloadPath}.sig`, "utf8"),
			fixtures[0].publicKey,
			version,
		),
	).toThrow("signed version");
});
