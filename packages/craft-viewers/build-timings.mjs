import { randomUUID } from "node:crypto";
import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const phases = ["prepare", "compile", "package", "stage"];

export function createBuildTimings({
	id,
	env = process.env,
	now = () => performance.now(),
}) {
	if (typeof id !== "string" || !/^[a-z0-9][a-z0-9_-]*$/.test(id))
		throw new Error(
			"Build timing id must contain only lowercase letters, digits, underscores or hyphens.",
		);
	const configuredDirectory = env.CRAFT_BUILD_REPORT_DIR;
	if (
		configuredDirectory !== undefined &&
		typeof configuredDirectory !== "string"
	)
		throw new Error("CRAFT_BUILD_REPORT_DIR must be a directory path.");
	const directory = configuredDirectory ? resolve(configuredDirectory) : null;
	const started = now();
	const phaseMs = Object.fromEntries(phases.map((phase) => [phase, 0]));
	let activePhase;
	let phaseStarted = started;
	let report;

	function recordPhase(ended) {
		if (activePhase) phaseMs[activePhase] += ended - phaseStarted;
		phaseStarted = ended;
	}

	return {
		phase(name) {
			if (report) throw new Error("Build timings have already finished.");
			if (!phases.includes(name))
				throw new Error(`Unknown build timing phase: ${name}`);
			recordPhase(now());
			activePhase = name;
		},
		finish({ cached = false, success = false, ...metadata } = {}) {
			if (report) return report;
			const ended = now();
			recordPhase(ended);
			report = {
				...Object.fromEntries(
					Object.entries(metadata).filter(([, value]) => value !== undefined),
				),
				schemaVersion: 1,
				id,
				totalMs: ended - started,
				phaseMs: { ...phaseMs },
				cached,
				success,
			};
			if (directory) {
				mkdirSync(directory, { recursive: true });
				const temporary = join(directory, `.${id}.${randomUUID()}.tmp`);
				try {
					writeFileSync(temporary, `${JSON.stringify(report, null, 2)}\n`, {
						flag: "wx",
					});
					renameSync(temporary, join(directory, `${id}.json`));
				} finally {
					rmSync(temporary, { force: true });
				}
			}
			return report;
		},
	};
}
