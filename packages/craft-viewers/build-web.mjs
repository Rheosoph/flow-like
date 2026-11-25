import { spawn } from "node:child_process";
import {
	closeSync,
	existsSync,
	mkdirSync,
	openSync,
	readdirSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBuildLock } from "./build-lock.mjs";
import { buildUsage, parseBuildOptions } from "./build-options.mjs";
import { buildConfiguration, runBuilds } from "./build-pool.mjs";
import { createBuildTimings } from "./build-timings.mjs";

const script = fileURLToPath(import.meta.url);
const directory = dirname(script);
const root = resolve(directory, "../..");
const development = join(root, "target/craft-viewers/development");

async function main() {
	const ids = [
		"wordcraft",
		...readdirSync(directory)
			.sort()
			.filter((id) => existsSync(join(directory, id, "upstream.json"))),
	];
	const options = parseBuildOptions(process.argv.slice(2), { ids });
	if (options.help) {
		console.log(buildUsage(ids));
		return;
	}
	const configuration = buildConfiguration({ env: options.env });
	if (options.development) {
		mkdirSync(development, { recursive: true });
		const log = join(development, "build.log");
		const descriptor = openSync(log, "a");
		const child = spawn(process.execPath, [script, ...options.backgroundArgs], {
			cwd: root,
			env: configuration.env,
			detached: true,
			stdio: ["ignore", descriptor, descriptor],
		});
		closeSync(descriptor);
		child.unref();
		console.log(
			`File editors prepare in the background (${options.cache} cache). Build log: ${log}`,
		);
		return;
	}

	const started = performance.now();
	const timings = createBuildTimings({ id: "suite", env: configuration.env });
	let release;
	let success = false;
	let skipped = false;
	try {
		if (options.background) {
			release = acquireBuildLock({
				directory: development,
				label: "file editor suite",
				wait: false,
			});
			if (release === null) {
				skipped = true;
				console.log(
					"An active development build already holds the file editor suite lock.",
				);
				return;
			}
		}
		const tasks = options.ids.map((id) => ({
			label: id,
			args:
				id === "wordcraft"
					? [join(root, "packages/wordcraft/build-web.mjs")]
					: [join(directory, "build-runtime.mjs"), id],
		}));
		console.log(
			`Preparing ${tasks.length} file editors with up to ${configuration.concurrency} worker${configuration.concurrency === 1 ? "" : "s"} (${configuration.env.CARGO_BUILD_JOBS} Cargo jobs per build; ${options.cache} cache).`,
		);
		await runBuilds(tasks, { ...configuration, cwd: root });
		success = true;
	} finally {
		release?.();
		if (!skipped) {
			console.log(
				`File editor suite ${success ? "completed" : "failed"} in ${((performance.now() - started) / 1000).toFixed(1)}s elapsed.`,
			);
			try {
				timings.finish({
					selectedIds: options.ids,
					concurrency: configuration.concurrency,
					cargoJobs: configuration.env.CARGO_BUILD_JOBS,
					cacheStrategy: options.cache,
					rebuild: configuration.env.CRAFT_REBUILD === "1",
					stageEnabled: configuration.env.CRAFT_SKIP_STAGE !== "1",
					success,
				});
			} catch (error) {
				console.error(
					`Could not write the file editor suite report: ${error.message}`,
				);
				process.exitCode = 1;
			}
		}
	}
}

try {
	await main();
} catch (error) {
	console.error(error.message);
	process.exitCode = 1;
}
