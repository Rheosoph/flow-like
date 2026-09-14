#!/usr/bin/env bun
// Read pending compute attempts and AWS REPORT events. No customer quota is changed.
// FLOW_LIKE_API_URL and MAINTENANCE_TOKEN are required. AWS CLI uses its normal profile.
// Defaults to a dry run; --apply records AWS billed-duration estimates. Optional
// --cursor-file persists scan progress for a scheduler. Maximum 100 attempts per run.
// Platform event schema: https://docs.aws.amazon.com/lambda/latest/dg/telemetry-schema-reference.html
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";

interface PendingAttempt {
	id: string;
	functionName: string;
	requestId: string;
	operationId: string | null;
	payerId: string | null;
	role: string;
	costClass: string;
	memoryMb: number;
	architecture: string;
	region: string;
	measuredDurationMs: number | null;
	status: string;
	startedAt: string;
}

export interface AwsReport {
	requestId: string;
	durationMs: number;
	billedDurationMs: number;
	memoryMb: number;
	status: "completed" | "failed" | "timeout" | "unknown";
}

function validReport(report: AwsReport): AwsReport | null {
	return /^[a-zA-Z0-9-]{1,128}$/.test(report.requestId) &&
		Number.isFinite(report.durationMs) &&
		report.durationMs >= 0 &&
		Number.isSafeInteger(report.billedDurationMs) &&
		report.billedDurationMs >= 0 &&
		Number.isSafeInteger(report.memoryMb) &&
		report.memoryMb >= 128 &&
		report.memoryMb <= 10240
		? report
		: null;
}

function reportStatus(status: unknown): AwsReport["status"] {
	if (status === "success") return "completed";
	if (status === "error" || status === "failure") return "failed";
	if (status === "timeout") return "timeout";
	return "unknown";
}

export function parseAwsReport(message: string): AwsReport | null {
	if (message.trimStart().startsWith("{")) {
		try {
			const value = JSON.parse(message);
			if (value?.type !== "platform.report") return null;
			const record = value.record;
			return validReport({
				requestId: record.requestId,
				durationMs: record.metrics.durationMs,
				billedDurationMs: record.metrics.billedDurationMs,
				memoryMb: record.metrics.memorySizeMB,
				status: reportStatus(record.status),
			});
		} catch {
			return null;
		}
	}
	// Anchor at REPORT: ordinary application logs must never supply billing evidence.
	const match = message
		.trim()
		.match(
			/^REPORT\s+RequestId:\s+([a-zA-Z0-9-]+)\s+Duration:\s+([\d.]+)\s+ms\s+Billed Duration:\s+(\d+)\s+ms\s+Memory Size:\s+(\d+)\s+MB\b/,
		);
	if (!match) return null;
	return validReport({
		requestId: match[1],
		durationMs: Number(match[2]),
		billedDurationMs: Number(match[3]),
		memoryMb: Number(match[4]),
		status: reportStatus(message.match(/\bStatus:\s+(\w+)/)?.[1]),
	});
}

export function buildRevision(attempt: PendingAttempt, report: AwsReport) {
	if (
		attempt.requestId !== report.requestId ||
		attempt.memoryMb !== report.memoryMb
	) {
		throw new Error(
			"AWS REPORT identity or memory does not match the recorded attempt",
		);
	}
	const { id: _, ...metadata } = attempt;
	return {
		...metadata,
		// Preserve handler measurements; REPORT duration is retained as evidence in the revision ID.
		measuredDurationMs: attempt.measuredDurationMs,
		billedDurationMs: report.billedDurationMs,
		costMicroUsd: null,
		evidence: "aws_report_estimate",
		rateVersion: "aws-lambda-reference-2026-09",
		status:
			report.status === "unknown"
				? attempt.status === "started"
					? "unknown"
					: attempt.status
				: report.status,
		revision: `aws-report:${createHash("sha256").update(JSON.stringify(report)).digest("hex")}`,
	};
}

function option(args: string[], name: string): string | undefined {
	const index = args.indexOf(name);
	if (index < 0) return undefined;
	const value = args[index + 1];
	if (!value || value.startsWith("--"))
		throw new Error(`${name} requires a value`);
	return value;
}

async function main() {
	const args = process.argv.slice(2);
	if (args.includes("--help")) {
		console.log(
			"reconcile-compute-attempts.ts [--apply] [--limit 1..100] [--cursor-file PATH]\nReads AWS CloudWatch REPORT events. Records estimates only with --apply; never adjusts customer quotas.",
		);
		return;
	}
	const api = process.env.FLOW_LIKE_API_URL;
	const token = process.env.MAINTENANCE_TOKEN;
	if (!api || !token)
		throw new Error("Set FLOW_LIKE_API_URL and MAINTENANCE_TOKEN");
	const base = new URL(api);
	if (
		base.username ||
		base.password ||
		base.search ||
		base.hash ||
		(base.protocol !== "https:" &&
			!(
				base.protocol === "http:" &&
				["localhost", "127.0.0.1"].includes(base.hostname)
			))
	) {
		throw new Error(
			"Use an HTTPS API origin, or HTTP localhost for development",
		);
	}
	const limit = Number(option(args, "--limit") ?? 25);
	if (!Number.isInteger(limit) || limit < 1 || limit > 100)
		throw new Error("--limit must be 1..100");
	const apply = args.includes("--apply");
	const cursorFile = option(args, "--cursor-file");
	const cursor =
		cursorFile && existsSync(cursorFile)
			? readFileSync(cursorFile, "utf8").trim()
			: "";
	if (cursor && !/^[a-f0-9]{64}$/.test(cursor))
		throw new Error("Invalid saved cursor");
	const request = async (path: string, body?: unknown) => {
		const result = await fetch(
			new URL(`/api/v1/maintenance/compute-attempts/${path}`, base),
			{
				method: body ? "POST" : "GET",
				redirect: "error",
				signal: AbortSignal.timeout(30_000),
				headers: {
					Authorization: `Bearer ${token}`,
					"Content-Type": "application/json",
				},
				body: body ? JSON.stringify(body) : undefined,
			},
		);
		if (!result.ok)
			throw new Error(`Maintenance API returned HTTP ${result.status}`);
		return result.json();
	};
	const page: { items: PendingAttempt[]; nextCursor: string | null } =
		await request(`pending?limit=${limit}${cursor ? `&cursor=${cursor}` : ""}`);
	if (!Array.isArray(page.items) || page.items.length > limit)
		throw new Error("Unexpected pending response");
	const reports = [];
	let missing = 0;
	for (const attempt of page.items) {
		if (
			!/^[a-zA-Z0-9-_]{1,64}$/.test(attempt.functionName) ||
			!/^[a-zA-Z0-9-]{1,128}$/.test(attempt.requestId) ||
			!/^[a-z]{2}-[a-z]+-\d+$/.test(attempt.region)
		) {
			throw new Error("Recorded attempt has an unsupported AWS identity");
		}
		const start = Date.parse(attempt.startedAt);
		if (!Number.isFinite(start)) throw new Error("Invalid attempt timestamp");
		// Allow the longest ordinary Lambda run plus CloudWatch delivery delay before scanning.
		if (Date.now() - start < 20 * 60_000) {
			missing++;
			continue;
		}
		let nextToken: string | undefined;
		let found: AwsReport | null = null;
		for (let pageIndex = 0; pageIndex < 2; pageIndex++) {
			const command = [
				"logs",
				"filter-log-events",
				"--region",
				attempt.region,
				"--log-group-name",
				`/aws/lambda/${attempt.functionName}`,
				"--filter-pattern",
				`"${attempt.requestId}"`,
				"--start-time",
				String(start - 60_000),
				"--end-time",
				String(start + 20 * 60_000),
				"--limit",
				"1000",
				"--no-paginate",
				"--output",
				"json",
				"--no-cli-pager",
			];
			if (nextToken) command.push("--next-token", nextToken);
			const result = spawnSync("aws", command, {
				encoding: "utf8",
				timeout: 30_000,
				maxBuffer: 2 * 1024 * 1024,
				env: { ...process.env, AWS_PAGER: "" },
			});
			if (result.status !== 0)
				throw new Error("AWS log read failed; scan cursor was not advanced");
			const data = JSON.parse(result.stdout);
			for (const event of data.events ?? []) {
				const parsed =
					typeof event.message === "string"
						? parseAwsReport(event.message)
						: null;
				if (parsed?.requestId === attempt.requestId) {
					found = parsed;
					break;
				}
			}
			if (found || !data.nextToken || data.nextToken === nextToken) break;
			nextToken = data.nextToken;
		}
		if (found) reports.push(buildRevision(attempt, found));
		else missing++;
	}
	if (apply && reports.length)
		await request("reconcile", { attempts: reports });
	if (apply && cursorFile) {
		const temporary = `${cursorFile}.tmp`;
		writeFileSync(temporary, page.nextCursor ?? "", { mode: 0o600 });
		renameSync(temporary, cursorFile);
	}
	console.log(
		JSON.stringify({
			mode: apply ? "apply" : "dry-run",
			examined: page.items.length,
			reportMatches: reports.length,
			pending: missing,
			nextCursor: page.nextCursor,
		}),
	);
}

if (import.meta.main)
	main().catch(() => {
		// Provider and API errors can contain request metadata. Keep scheduler logs free of secrets.
		console.error(
			"Compute reconciliation failed. Check configuration and AWS access; customer quotas were not adjusted.",
		);
		process.exitCode = 1;
	});
