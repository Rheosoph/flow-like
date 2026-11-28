import assert from "node:assert/strict";
import {
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createBuildTimings } from "./build-timings.mjs";

function clock() {
	let time = 100;
	return {
		now: () => time,
		advance: (elapsed) => {
			time += elapsed;
		},
	};
}

test("records phase boundaries and accumulates repeated phases", () => {
	const time = clock();
	const timings = createBuildTimings({
		id: "gridcraft",
		env: {},
		now: time.now,
	});
	time.advance(2);
	timings.phase("prepare");
	time.advance(3);
	timings.phase("compile");
	time.advance(11);
	timings.phase("package");
	time.advance(5);
	timings.phase("prepare");
	time.advance(7);
	timings.phase("stage");
	time.advance(13);
	const report = timings.finish({ success: true });
	assert.equal(report.totalMs, 41);
	assert.deepEqual(report.phaseMs, {
		prepare: 10,
		compile: 11,
		package: 5,
		stage: 13,
	});
	assert.equal(report.success, true);
	assert.equal(report.cached, false);
});

test("a failed phase is timed without replacing the build error", () => {
	const time = clock();
	const timings = createBuildTimings({
		id: "wordcraft",
		env: {},
		now: time.now,
	});
	const failure = new Error("compiler failed");
	let report;
	assert.throws(
		() => {
			timings.phase("compile");
			try {
				time.advance(23);
				throw failure;
			} finally {
				report = timings.finish({ success: false });
			}
		},
		(error) => error === failure,
	);
	assert.equal(report.success, false);
	assert.equal(report.totalMs, 23);
	assert.equal(report.phaseMs.compile, 23);
});

test("finishing is stable and reports cached builds with no compile time", () => {
	const time = clock();
	const timings = createBuildTimings({
		id: "deckcraft",
		env: {},
		now: time.now,
	});
	timings.phase("stage");
	time.advance(4);
	const report = timings.finish({ cached: true, success: true });
	time.advance(100);
	assert.equal(timings.finish({ success: false }), report);
	assert.equal(report.totalMs, 4);
	assert.equal(report.phaseMs.compile, 0);
	assert.equal(report.cached, true);
	assert.throws(() => timings.phase("compile"), /already finished/);
});

test("writes one complete report per editor and replaces an earlier report", () => {
	const temporary = mkdtempSync(join(tmpdir(), "flow-craft-timings-"));
	const directory = join(temporary, "reports", "run");
	try {
		const time = clock();
		const env = { CRAFT_BUILD_REPORT_DIR: directory };
		const timings = createBuildTimings({
			id: "photocraft",
			env,
			now: time.now,
		});
		timings.phase("prepare");
		time.advance(9);
		const report = timings.finish({
			success: true,
			revision: "abc123",
			cacheStrategy: "shared",
			wasmBytes: 1024,
			compressedBytes: 512,
			absent: undefined,
			id: "override",
			schemaVersion: 99,
			totalMs: 999,
			phaseMs: {},
		});
		const path = join(directory, "photocraft.json");
		assert.deepEqual(JSON.parse(readFileSync(path, "utf8")), report);
		assert.equal(report.id, "photocraft");
		assert.equal(report.schemaVersion, 1);
		assert.equal(report.totalMs, 9);
		assert.equal(report.phaseMs.prepare, 9);
		assert.equal(report.wasmBytes, 1024);
		assert.equal(report.compressedBytes, 512);
		assert.equal("absent" in report, false);
		writeFileSync(path, "stale");
		const replacement = createBuildTimings({ id: "photocraft", env }).finish({
			cached: true,
			success: true,
		});
		assert.deepEqual(JSON.parse(readFileSync(path, "utf8")), replacement);
		assert.deepEqual(readdirSync(directory), ["photocraft.json"]);
	} finally {
		rmSync(temporary, { recursive: true, force: true });
	}
});

test("rejects path-like ids and unknown phases before writing", () => {
	for (const id of ["", "../wordcraft", "a/b", "a\\b", ".", undefined])
		assert.throws(() => createBuildTimings({ id, env: {} }), /timing id/);
	const timings = createBuildTimings({ id: "pdfcraft", env: {} });
	assert.throws(() => timings.phase("compiel"), /Unknown build timing phase/);
	assert.throws(
		() =>
			createBuildTimings({
				id: "pdfcraft",
				env: { CRAFT_BUILD_REPORT_DIR: 4 },
			}),
		/CRAFT_BUILD_REPORT_DIR/,
	);
});

test("a failed report replacement leaves no partial report behind", () => {
	const directory = mkdtempSync(join(tmpdir(), "flow-craft-timings-failure-"));
	try {
		mkdirSync(join(directory, "vectorcraft.json"));
		const timings = createBuildTimings({
			id: "vectorcraft",
			env: { CRAFT_BUILD_REPORT_DIR: directory },
		});
		assert.throws(() => timings.finish({ success: false }));
		assert.deepEqual(readdirSync(directory), ["vectorcraft.json"]);
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
});
