import { spawn } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { availableParallelism } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const uiRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(uiRoot, "../..");

// Files that cannot pass yet. Each needs a reason; nothing is excluded silently.
const EXCLUDED = new Map([]);

const SKIPPED_DIRS = new Set(["node_modules", ".next", "dist", "out"]);
const TEST_FILE = /\.test\.tsx?$/;
const VITEST_IMPORT = /from\s+["']vitest["']/;
const jobs = Number(process.env.UI_TEST_JOBS) || Math.min(8, availableParallelism());
const FILE_TIMEOUT_MS = 180_000;
const only =process.argv.slice(2);

function collect(dir, found = []) {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.isDirectory()) {
			if (!SKIPPED_DIRS.has(entry.name)) collect(path.join(dir, entry.name), found);
		} else if (TEST_FILE.test(entry.name)) {
			found.push(path.relative(uiRoot, path.join(dir, entry.name)));
		}
	}
	return found;
}

function run(command, args, cwd, label, timeoutMs = 0) {
	return new Promise((resolve) => {
		const chunks = [];
		const started = Date.now();
		const child = spawn(command, args, { cwd, env: process.env });
		const timer = timeoutMs
			? setTimeout(() => {
					chunks.push(Buffer.from(`\nkilled after ${timeoutMs / 1000}s without finishing\n`));
					child.kill("SIGKILL");
				}, timeoutMs)
			: undefined;
		child.on("close", () => clearTimeout(timer));
		child.stdout.on("data", (data) => chunks.push(data));
		child.stderr.on("data", (data) => chunks.push(data));
		child.on("error", (error) => {
			chunks.push(Buffer.from(String(error)));
			resolve({ label, code: 1, output: Buffer.concat(chunks).toString(), ms: 0 });
		});
		child.on("close", (code) =>
			resolve({
				label,
				code: code ?? 1,
				output: Buffer.concat(chunks).toString(),
				ms: Date.now() - started,
			}),
		);
	});
}

async function pool(items, worker) {
	const results = [];
	let next = 0;
	const lanes = Array.from({ length: Math.min(jobs, items.length) }, async () => {
		while (next < items.length) {
			results.push(await worker(items[next++]));
		}
	});
	await Promise.all(lanes);
	return results;
}

function summarize(output) {
	const lines = output.split("\n").filter((line) => /^\s*(\d+ (pass|fail)|Tests\s|Test Files\s)/.test(line));
	return lines.join(" | ");
}

const all = collect(uiRoot)
	.filter((file) => !EXCLUDED.has(file))
	.filter((file) => only.length === 0 || only.some((prefix) => file.startsWith(prefix)))
	.sort();
const vitestFiles = [];
const bunFiles = [];
for (const file of all) {
	const source = readFileSync(path.join(uiRoot, file), "utf8");
	if (VITEST_IMPORT.test(source)) vitestFiles.push(file);
	else bunFiles.push(file);
}

console.log(
	`ui tests: ${bunFiles.length} bun files (one process each, ${jobs} parallel), ${vitestFiles.length} vitest files. Excluded: ${EXCLUDED.size}`,
);

const failures = [];
const started = Date.now();

function report(result) {
	const state = result.code === 0 ? "ok  " : "FAIL";
	console.log(`${state} ${result.label} (${(result.ms / 1000).toFixed(1)}s) ${summarize(result.output)}`);
	if (result.code !== 0) failures.push(result);
}

const bunResults = await pool(bunFiles, (file) =>
	run("bun", ["test", "--timeout", "30000", `./${file}`], uiRoot, file, FILE_TIMEOUT_MS),
);
for (const result of bunResults.sort((a, b) => a.label.localeCompare(b.label))) report(result);

if (vitestFiles.length > 0) {
	const args = [
		"vitest",
		"run",
		"--environment",
		"happy-dom",
		"--maxWorkers",
		String(Math.max(1, Math.min(4, jobs))),
		...vitestFiles.map((file) => `packages/ui/${file}`),
	];
	report(await run("bunx", args, repoRoot, "vitest"));
}

for (const failure of failures) {
	console.log(`\n===== ${failure.label} =====\n${failure.output.split("\n").slice(-80).join("\n")}`);
}
console.log(
	`\nui tests finished in ${((Date.now() - started) / 1000).toFixed(0)}s, ${failures.length} failing group(s)${failures.length ? `: ${failures.map((failure) => failure.label).join(", ")}` : ""}`,
);
process.exit(failures.length === 0 ? 0 : 1);
