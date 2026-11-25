import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	copyFileSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parseBuildOptions } from "./build-options.mjs";

const ids = ["wordcraft", "gridcraft", "photocraft"];

test("default options keep every editor and share the Cargo cache", () => {
	const options = parseBuildOptions([], { ids, env: {} });
	assert.deepEqual(options.ids, ids);
	assert.equal(options.cache, "shared");
	assert.equal(options.env.CRAFT_BUILD_CACHE, "shared");
	assert.equal(options.env.CRAFT_REBUILD, undefined);
	assert.equal(options.env.CRAFT_SKIP_STAGE, undefined);
	assert.equal(options.env.CRAFT_BUILD_REPORT_DIR, undefined);
});

test("selects distinct editors in requested order and preserves unrelated environment", () => {
	const env = { CRAFT_BUILD_CACHE: "shared", CARGO_BUILD_JOBS: "2" };
	const options = parseBuildOptions(["photocraft", "wordcraft", "photocraft"], {
		ids,
		env,
	});
	assert.deepEqual(options.ids, ["photocraft", "wordcraft"]);
	assert.equal(options.cache, "shared");
	assert.equal(options.env.CARGO_BUILD_JOBS, "2");
	assert.notEqual(options.env, env);
	assert.deepEqual(env, { CRAFT_BUILD_CACHE: "shared", CARGO_BUILD_JOBS: "2" });
});

test("background forwarding retains the request and resolves reports before changing directories", () => {
	const cwd = join(tmpdir(), "flow-craft-options");
	const options = parseBuildOptions(
		[
			"--dev",
			"--cache",
			"shared",
			"--report",
			"reports/run",
			"--rebuild",
			"--no-stage",
			"gridcraft",
		],
		{ ids, cwd, env: { CRAFT_BUILD_CACHE: "auto" } },
	);
	assert.equal(options.development, true);
	assert.equal(options.background, false);
	assert.equal(options.env.CRAFT_REBUILD, "1");
	assert.equal(options.env.CRAFT_SKIP_STAGE, "1");
	assert.equal(options.env.CRAFT_BUILD_REPORT_DIR, resolve(cwd, "reports/run"));
	const child = parseBuildOptions(options.backgroundArgs, {
		ids,
		cwd: join(tmpdir(), "another-directory"),
		env: options.env,
	});
	assert.equal(child.development, false);
	assert.equal(child.background, true);
	assert.deepEqual(child.ids, ["gridcraft"]);
	assert.deepEqual(child.env, options.env);
});

test("normalizes inherited report directories and permits CLI cache overrides", () => {
	const cwd = join(tmpdir(), "flow-craft-options");
	const options = parseBuildOptions(["--cache", "auto"], {
		ids,
		cwd,
		env: { CRAFT_BUILD_CACHE: "shared", CRAFT_BUILD_REPORT_DIR: "reports" },
	});
	assert.equal(options.cache, "auto");
	assert.equal(options.env.CRAFT_BUILD_REPORT_DIR, resolve(cwd, "reports"));
});

test("rejects malformed options and unknown editors", () => {
	for (const args of [
		["--cache"],
		["--cache", "--rebuild"],
		["--cache", "isolated"],
		["--report"],
		["--report", "--no-stage"],
		["--unknown"],
		["unknowncraft"],
		["--dev", "--background"],
	])
		assert.throws(() => parseBuildOptions(args, { ids, env: {} }));
	assert.throws(
		() => parseBuildOptions([], { ids, env: { CRAFT_BUILD_CACHE: "invalid" } }),
		/CRAFT_BUILD_CACHE/,
	);
});

test("CLI help and invalid requests never create reports or start editor builds", () => {
	const temporary = mkdtempSync(join(tmpdir(), "flow-craft-options-cli-"));
	const report = join(temporary, "reports");
	const script = fileURLToPath(new URL("./build-web.mjs", import.meta.url));
	try {
		const help = spawnSync(
			process.execPath,
			[script, "--help", "--report", report],
			{ encoding: "utf8", timeout: 5000 },
		);
		assert.equal(help.status, 0, help.stderr);
		assert.match(help.stdout, /Usage:/);
		assert.match(help.stdout, /Compile timing includes Cargo lock/);
		assert.equal(existsSync(report), false);
		const invalid = spawnSync(
			process.execPath,
			[script, "--report", report, "unknowncraft"],
			{ encoding: "utf8", timeout: 5000 },
		);
		assert.equal(invalid.status, 1);
		assert.match(invalid.stderr, /Unknown editor: unknowncraft/);
		assert.equal(existsSync(report), false);
	} finally {
		rmSync(temporary, { recursive: true, force: true });
	}
});

test("suite reports selected workers and preserves their failure status", () => {
	const temporary = mkdtempSync(join(tmpdir(), "flow-craft-suite-"));
	const directory = join(temporary, "packages", "craft-viewers");
	try {
		mkdirSync(join(directory, "gridcraft"), { recursive: true });
		writeFileSync(join(directory, "gridcraft", "upstream.json"), "{}");
		for (const file of [
			"build-web.mjs",
			"build-options.mjs",
			"build-pool.mjs",
			"build-lock.mjs",
			"build-timings.mjs",
		])
			copyFileSync(new URL(file, import.meta.url), join(directory, file));
		writeFileSync(
			join(directory, "build-runtime.mjs"),
			`if (process.argv[2] !== "gridcraft" ||
process.env.CRAFT_BUILD_CACHE !== "shared" ||
process.env.CRAFT_REBUILD !== "1" ||
process.env.CRAFT_SKIP_STAGE !== "1") process.exit(5);
process.exit(Number(process.env.CRAFT_TEST_EXIT));`,
		);
		for (const code of [0, 7]) {
			const reportDirectory = join("reports", String(code));
			const child = spawnSync(
				process.execPath,
				[
					join(directory, "build-web.mjs"),
					"--report",
					reportDirectory,
					"--rebuild",
					"--no-stage",
					"gridcraft",
				],
				{
					cwd: temporary,
					env: {
						...process.env,
						CRAFT_BUILD_CACHE: undefined,
						CRAFT_BUILD_CONCURRENCY: "1",
						CARGO_BUILD_JOBS: "2",
						CRAFT_TEST_EXIT: String(code),
					},
					encoding: "utf8",
					timeout: 5000,
				},
			);
			assert.equal(child.status, code === 0 ? 0 : 1, child.stderr);
			assert.match(child.stdout, /shared cache/);
			assert.match(child.stdout, /suite (completed|failed) in .*s elapsed/);
			const report = JSON.parse(
				readFileSync(join(temporary, reportDirectory, "suite.json"), "utf8"),
			);
			assert.deepEqual(report.selectedIds, ["gridcraft"]);
			assert.equal(report.concurrency, 1);
			assert.equal(report.cargoJobs, "2");
			assert.equal(report.cacheStrategy, "shared");
			assert.equal(report.rebuild, true);
			assert.equal(report.stageEnabled, false);
			assert.equal(report.success, code === 0);
			assert.ok(report.totalMs > 0);
		}
	} finally {
		rmSync(temporary, { recursive: true, force: true });
	}
});
