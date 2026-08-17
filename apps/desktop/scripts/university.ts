#!/usr/bin/env bun

import { platform } from "node:os";
import {
	type UniversityCliOptions,
	parseUniversityArgs,
} from "../lib/university/cli";
import { loadUniversityPlan } from "../lib/university/plan";
import {
	type UniversityCommandResult,
	inspectUniversityCourse,
	listUniversityCourses,
	planUniversityRun,
	runUniversityPlan,
	uploadSingleUniversityAsset,
} from "../lib/university/runner";

export function usage(): string {
	return `Flow-Like University management CLI

Usage:
  bun run university -- --plan path/to/course.plan.json --dry-run --json
  bun run university -- --plan path/to/course.plan.json --json
  bun run university -- --inspect <course-id> [--language en] --json
  bun run university -- --list [--language en] --json
  bun run university -- --asset <course-id> --name <AssetName> --file <path> [--replace] --json

Modes:
  --plan <path>          Validate and apply a flow-like.university-plan/v1 file
  --inspect <course-id>  Read the full course, including lessons and challenges
  --list                 List all courses, including drafts when permitted
  --asset <course-id>    Upload one file or screenshot as a named course asset

Plan and asset options:
  --dry-run              Validate a plan and print ordered operations without API access
  --name <name>          Asset reference name used as @Name in lesson Markdown
  --file <path>          Local asset file, including a doc-screenshot artifact path
  --kind <kind>          IMAGE, VIDEO, AUDIO, or DOCUMENT; inferred when omitted
  --mime-type <type>     MIME type; inferred when omitted
  --replace              Force replacement of a same-named asset

Connection and output:
  --api-url <url>        API origin; defaults to FLOW_LIKE_BASE_URL
  --timeout-ms <number>  Whole-command timeout, at most 300000 (default: 120000)
  --language <code>      Language used by list or inspect
  --json                 Reserve stdout for one versioned JSON result
  --help                 Show this help

Authentication is read only from FLOW_LIKE_PAT. The PAT owner needs the
WriteCourses global permission for authoring. API keys are not accepted.`;
}

function remoteOptions(options: UniversityCliOptions): {
	apiUrl: string;
	pat: string;
	timeoutMs: number;
} {
	const apiUrl = options.apiUrl ?? process.env.FLOW_LIKE_BASE_URL;
	if (!apiUrl) {
		throw new Error(
			"Set FLOW_LIKE_BASE_URL or pass --api-url for University API access.",
		);
	}
	const pat = process.env.FLOW_LIKE_PAT;
	if (!pat) {
		throw new Error("Set FLOW_LIKE_PAT for University API access.");
	}
	if (!pat.startsWith("pat_")) {
		throw new Error("FLOW_LIKE_PAT must be a Flow-Like personal access token.");
	}
	return { apiUrl, pat, timeoutMs: options.timeoutMs ?? 120_000 };
}

function printResult(result: UniversityCommandResult, json: boolean): void {
	if (json) {
		console.log(JSON.stringify(result));
		return;
	}
	const outcome = result.passed ? "PASS" : "FAIL";
	console.log(
		`${outcome} University ${result.command} in ${(result.durationMs / 1000).toFixed(1)}s`,
	);
	if (result.courseId) console.log(`  course: ${result.courseId}`);
	if (result.summary) console.log(`  ${result.summary}`);
	if (result.error) console.log(`  ${result.error}`);
	for (const operation of result.operations ?? []) {
		console.log(`  ${operation.status.padEnd(9)} ${operation.description}`);
	}
}

async function run(options: UniversityCliOptions): Promise<number> {
	let result: UniversityCommandResult;
	switch (options.mode) {
		case "apply": {
			const plan = await loadUniversityPlan(options.plan ?? "");
			result = options.dryRun
				? planUniversityRun(plan)
				: await runUniversityPlan(plan, remoteOptions(options));
			break;
		}
		case "inspect":
			result = await inspectUniversityCourse(
				options.inspectCourseId ?? "",
				options.language,
				remoteOptions(options),
			);
			break;
		case "list":
			result = await listUniversityCourses(
				options.language,
				remoteOptions(options),
			);
			break;
		case "asset":
			result = await uploadSingleUniversityAsset(
				{
					courseId: options.assetCourseId ?? "",
					name: options.assetName ?? "",
					file: options.assetFile ?? "",
					kind: options.assetKind,
					mimeType: options.assetMimeType,
					replace: options.replaceAsset,
				},
				remoteOptions(options),
			);
			break;
		default:
			throw new Error("No University command selected.");
	}
	printResult(result, options.json);
	return result.passed ? 0 : 1;
}

export async function main(args = Bun.argv.slice(2)): Promise<number> {
	let options: UniversityCliOptions;
	try {
		options = parseUniversityArgs(args);
	} catch (error) {
		console.error(error instanceof Error ? error.message : String(error));
		console.error("\nRun with --help for usage.");
		return 2;
	}
	if (options.help) {
		console.log(usage());
		return 0;
	}

	try {
		return await run(options);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		if (options.json) {
			console.log(
				JSON.stringify({
					schema: "flow-like.university-cli-error/v1",
					passed: false,
					error: message,
					platform: platform(),
				}),
			);
		} else {
			console.error(`University command failed: ${message}`);
		}
		return 2;
	}
}

if (import.meta.main) {
	process.exitCode = await main();
}
