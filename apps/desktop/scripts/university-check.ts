#!/usr/bin/env bun

import { fileURLToPath } from "node:url";
import { checkCurriculum } from "../lib/university/curriculum";
import { applyLearningPaths } from "../lib/university/learning-paths";
import { compareCurriculumRef } from "../lib/university/migration";

const usage = `Validate University courses and optionally publish ordered learning paths.

Usage:
  bun run university:check
  bun run university:check -- --json
  bun run university:check -- --catalog path/to/curriculum.json
  bun run university:check -- --apply-paths --json
  bun run university:check -- --compare-ref HEAD --json

Course plan references are relative to the catalog; courses/ is scanned recursively.
Without --apply-paths no API requests are made. Publishing reads FLOW_LIKE_BASE_URL
and FLOW_LIKE_PAT from the environment and requires WriteCourses permission.
Courses must already be published remotely. Unexpected steps are never deleted.
--compare-ref reports removed/moved child IDs from Git; it cannot be combined with --apply-paths.
Exit codes: 0 passed, 1 validation/apply failed, 2 invalid arguments or configuration.`;

function remoteOptions() {
	const apiUrl = process.env.FLOW_LIKE_BASE_URL;
	const pat = process.env.FLOW_LIKE_PAT;
	if (!apiUrl || !pat)
		throw new Error(
			"Set FLOW_LIKE_BASE_URL and FLOW_LIKE_PAT to publish learning paths.",
		);
	return { apiUrl, pat, timeoutMs: 120_000 };
}

export async function main(args = Bun.argv.slice(2)): Promise<number> {
	let catalog = fileURLToPath(
		new URL("../lib/university/curriculum.json", import.meta.url),
	);
	let json = false;
	let applyPaths = false;
	let compareRef: string | undefined;
	for (let index = 0; index < args.length; index++) {
		const arg = args[index];
		if (arg === "--") continue;
		if (arg === "--help") {
			console.log(usage);
			return 0;
		}
		if (arg === "--json") json = true;
		else if (arg === "--apply-paths") applyPaths = true;
		else if (
			(arg === "--catalog" || arg === "--compare-ref") &&
			args[index + 1] &&
			!args[index + 1].startsWith("--")
		) {
			const value = args[++index];
			if (arg === "--catalog") catalog = value;
			else compareRef = value;
		} else {
			console.error(`Unknown or incomplete argument: ${arg}\n${usage}`);
			return 2;
		}
	}
	if (applyPaths && compareRef) {
		console.error(
			"--compare-ref is read-only and cannot be combined with --apply-paths.",
		);
		return 2;
	}
	try {
		const baseResult = applyPaths
			? await applyLearningPaths(catalog, remoteOptions())
			: await checkCurriculum(catalog);
		const result = compareRef
			? {
					...baseResult,
					migration: await compareCurriculumRef(catalog, compareRef),
				}
			: baseResult;
		const check = "check" in result ? result.check : result;
		if (json) console.log(JSON.stringify(result));
		else {
			console.log(
				`${result.passed ? "PASS" : "FAIL"}: ${check.totals.courses} courses, ${check.totals.paths} learning paths`,
			);
			console.log(
				`${check.totals.requiredMinutes} required minutes + ${check.totals.optionalMinutes} optional minutes; ${check.totals.lessons} lessons; ${check.totals.questions} choice questions`,
			);
			console.log(
				`${check.totals.lessonWords} lesson words + ${check.totals.questionWords} question/answer words (approximate)`,
			);
			if ("check" in result)
				for (const path of result.paths)
					console.log(`  ${path.status}: ${path.id}`);
			if ("migration" in result) {
				console.log(
					`Migration from ${result.migration.ref}: ${result.migration.removed.length} removed entities, ${result.migration.moved.length} moved entities`,
				);
				for (const item of result.migration.removed)
					console.log(
						`  removed ${item.kind}: ${item.id} (${item.location.courseId})`,
					);
				for (const item of result.migration.moved)
					console.log(
						`  moved ${item.kind}: ${item.id} (${item.from.courseId} → ${item.to.courseId})`,
					);
			}
			for (const error of result.errors) console.error(error);
		}
		return result.passed ? 0 : 1;
	} catch (error) {
		const message = (
			error instanceof Error ? error.message : String(error)
		).replace(/pat_[A-Za-z0-9._-]+/g, "[REDACTED]");
		if (json)
			console.log(
				JSON.stringify({
					schema: "flow-like.university-check/v1",
					passed: false,
					errors: [message],
				}),
			);
		else console.error(message);
		return 2;
	}
}

if (import.meta.main) process.exit(await main());
