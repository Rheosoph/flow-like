#!/usr/bin/env bun

import { type ChildProcess, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdir, open, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createFlowPilotE2EIsolation } from "./flowpilot-e2e-isolation";

const desktopDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoDir = resolve(desktopDir, "../..");
const providers = ["claude-code", "codex"] as const;
const surfaces = [
	"flux",
	"flowpilot",
	"bot",
	"use-case",
	"invoke-model",
	"agent-from-model",
] as const;
export const requiredCases = providers.flatMap((provider) =>
	surfaces.map((surface) => `${provider}:${surface}`),
);

export interface LiveCase {
	id: string;
	provider: string;
	bitId: string;
	modelId: string;
	passed: boolean;
	evidence: Record<string, unknown>;
	error?: string;
}

export interface LiveReport {
	schema: "flowpilot.custom-bit-e2e/v1";
	runId: string;
	passed: boolean;
	cases: LiveCase[];
	progress?: { stage: string; caseId?: string };
}

export function validateReport(
	value: unknown,
	runId: string,
	final = true,
): LiveReport {
	if (!value || typeof value !== "object")
		throw new Error("Missing E2E report");
	const report = value as LiveReport;
	if (
		report.schema !== "flowpilot.custom-bit-e2e/v1" ||
		report.runId !== runId ||
		!Array.isArray(report.cases)
	)
		throw new Error("E2E report does not match this run");
	const ids = new Set<string>();
	const identities = new Map<string, string>();
	for (const item of report.cases) {
		if (
			!item ||
			!requiredCases.includes(item.id) ||
			ids.has(item.id) ||
			!item.id.startsWith(`${item.provider}:`) ||
			typeof item.passed !== "boolean"
		)
			throw new Error("Invalid or duplicate E2E case");
		ids.add(item.id);
		if (!item.passed) continue;
		if (
			!item.bitId ||
			!item.modelId ||
			item.modelId === "default" ||
			!item.evidence ||
			typeof item.evidence !== "object" ||
			Object.keys(item.evidence).length === 0
		)
			throw new Error(`Passing case ${item.id} has no concrete model evidence`);
		const identity = `${item.bitId}\n${item.modelId}`;
		if (
			identities.has(item.provider) &&
			identities.get(item.provider) !== identity
		)
			throw new Error(`Selected model changed between ${item.provider} cases`);
		identities.set(item.provider, identity);
	}
	const complete = requiredCases.every((id) => ids.has(id));
	if (final && !complete)
		throw new Error("E2E report omitted required execution paths");
	return {
		...report,
		passed: complete && report.cases.every((item) => item.passed),
	};
}

function childResult(child: ChildProcess): Promise<number> {
	return new Promise((done, reject) => {
		child.once("error", reject);
		child.once("exit", (code) => done(code ?? 1));
	});
}

async function stop(child: ChildProcess | undefined) {
	if (!child || child.exitCode !== null || child.signalCode !== null) return;
	child.kill("SIGTERM");
	await Promise.race([childResult(child), Bun.sleep(5_000)]);
	if (child.exitCode === null && child.signalCode === null)
		child.kill("SIGKILL");
}

export async function main(args: string[]) {
	let frontendUrl: string | undefined;
	let output: string | undefined;
	let skipBuild = false;
	let timeoutMs = 45 * 60_000;
	for (let index = 0; index < args.length; index++) {
		const arg = args[index];
		if (arg === "--frontend-url") frontendUrl = args[++index];
		else if (arg === "--output") output = args[++index];
		else if (arg === "--skip-build") skipBuild = true;
		else if (arg === "--timeout-ms") timeoutMs = Number(args[++index]);
		else if (arg === "--help") {
			console.log(
				"Usage: bun apps/desktop/scripts/custom-bit-e2e.ts [--frontend-url http://localhost:3000] [--output /tmp/result.json] [--skip-build] [--timeout-ms 2700000]\nRuns live Claude/Codex custom Bits through isolated desktop bots, workflow nodes, Flux and FlowPilot. Uses existing local provider sign-in and spends model budget. Builds through the shared Cargo queue with the ci profile; --skip-build reuses that binary.",
			);
			return 0;
		} else throw new Error(`Unknown argument: ${arg}`);
	}
	if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1)
		throw new Error("Invalid timeout");
	const frontend = new URL(frontendUrl ?? "http://localhost:3415");
	if (
		frontend.protocol !== "http:" ||
		frontend.hostname !== "localhost" ||
		frontend.username ||
		frontend.password
	)
		throw new Error(
			"The isolated desktop runner requires an http://localhost frontend",
		);
	const runId = `e2e_${Date.now()}_${randomBytes(8).toString("hex")}`;
	const isolation = await createFlowPilotE2EIsolation(runId);
	const artifacts = await mkdir(
		join(tmpdir(), `flow-like-custom-bit-results-${runId}`),
		{ recursive: true, mode: 0o700 },
	);
	if (!artifacts) throw new Error("Could not create E2E artifacts directory");
	const outputPath = resolve(output ?? join(artifacts, "result.json"));
	const nativeLog = await open(join(artifacts, "desktop.log"), "w", 0o600);
	const frontendLog = await open(join(artifacts, "frontend.log"), "w", 0o600);
	const nonce = randomBytes(24).toString("hex");
	let settle!: (value: LiveReport) => void;
	const result = new Promise<LiveReport>((done) => {
		settle = done;
	});
	const server = Bun.serve({
		hostname: "127.0.0.1",
		port: 0,
		maxRequestBodySize: 32 * 1024 * 1024,
		async fetch(request) {
			const url = new URL(request.url);
			const headers = {
				"access-control-allow-origin": frontend.origin,
				"access-control-allow-methods": "POST, OPTIONS",
				"access-control-allow-headers": "Content-Type",
			};
			if (
				url.pathname !== `/${nonce}` ||
				(request.headers.get("origin") &&
					request.headers.get("origin") !== frontend.origin)
			)
				return new Response("Forbidden", { status: 403 });
			if (request.method === "OPTIONS")
				return new Response(null, { status: 204, headers });
			if (request.method !== "POST")
				return new Response("Method not allowed", { status: 405, headers });
			try {
				const final = url.searchParams.get("progress") !== "1";
				const report = validateReport(await request.json(), runId, final);
				await writeFile(
					final ? outputPath : join(artifacts, "progress.json"),
					`${JSON.stringify(report, null, 2)}\n`,
					{ mode: 0o600 },
				);
				const stage = final
					? "Result"
					: report.progress
						? [report.progress.caseId, report.progress.stage]
								.filter(Boolean)
								.join(" ")
						: "Progress";
				console.error(
					`${stage}: ${report.cases.filter((item) => item.passed).length}/${report.cases.length} completed checks passed`,
				);
				if (final) settle(report);
				return Response.json({ accepted: true }, { headers });
			} catch (error) {
				console.error(`Rejected result: ${String(error)}`);
				return Response.json(
					{ error: String(error) },
					{ status: 400, headers },
				);
			}
		},
	});
	let frontendChild: ChildProcess | undefined;
	let desktopChild: ChildProcess | undefined;
	let timer: ReturnType<typeof setTimeout> | undefined;
	try {
		console.error(
			`Artifacts: ${artifacts}\nIsolated app data: ${isolation.root}`,
		);
		if (!frontendUrl) {
			frontendChild = spawn(
				"node",
				[
					resolve(repoDir, "node_modules/next/dist/bin/next"),
					"dev",
					"--turbopack",
					"--hostname",
					"127.0.0.1",
					"--port",
					frontend.port,
				],
				{
					cwd: desktopDir,
					env: {
						...process.env,
						NEXT_TELEMETRY_DISABLED: "1",
						FLOWPILOT_E2E_NO_DISK_CACHE: "1",
					},
					stdio: ["ignore", frontendLog.fd, frontendLog.fd],
				},
			);
		}
		const config = {
			...isolation.config,
			build: { beforeDevCommand: null, devUrl: frontend.origin },
		};
		const env = {
			...process.env,
			TAURI_CONFIG: JSON.stringify(config),
			...isolation.env,
			FLOWPILOT_E2E_CLI_RUN_ID: runId,
			NEXT_TELEMETRY_DISABLED: "1",
		};
		if (!skipBuild) {
			console.error(
				"Building the isolated desktop through the shared Cargo queue...",
			);
			const build = spawn(
				resolve(repoDir, "tools/cargo-queue/bin/cargo"),
				[
					"build",
					"-p",
					"flow-like-desktop",
					"--bin",
					"flow-like-desktop",
					"--profile",
					"ci",
				],
				{ cwd: repoDir, env, stdio: ["ignore", "inherit", "inherit"] },
			);
			if (await childResult(build)) throw new Error("Native E2E build failed");
		}
		const runner = new URL("/developer/flowpilot-e2e", frontend);
		runner.searchParams.set("customBits", "1");
		runner.searchParams.set("cli", "1");
		runner.searchParams.set("cliRunId", runId);
		runner.searchParams.set(
			"callback",
			`http://localhost:${server.port}/${nonce}`,
		);
		const readyDeadline = Date.now() + 5 * 60_000;
		while (true) {
			if (frontendChild && frontendChild.exitCode !== null)
				throw new Error(
					`Frontend exited; see ${join(artifacts, "frontend.log")}`,
				);
			try {
				const response = await fetch(runner, {
					signal: AbortSignal.timeout(15_000),
				});
				if (response.ok) break;
			} catch {}
			if (Date.now() >= readyDeadline)
				throw new Error(
					`Frontend did not become ready; see ${join(artifacts, "frontend.log")}`,
				);
			await Bun.sleep(1_000);
		}
		desktopChild = spawn(
			resolve(repoDir, "target/ci/flow-like-desktop"),
			["--flowpilot-e2e-cli"],
			{
				cwd: desktopDir,
				env: { ...env, FLOWPILOT_E2E_CLI_URL: runner.toString() },
				stdio: ["ignore", nativeLog.fd, nativeLog.fd],
			},
		);
		const report = await Promise.race([
			result,
			childResult(desktopChild).then((code) => {
				throw new Error(
					`Desktop exited before results (${code}); see ${join(artifacts, "desktop.log")}`,
				);
			}),
			new Promise<never>((_, reject) => {
				timer = setTimeout(
					() => reject(new Error(`Live test timed out; see ${artifacts}`)),
					timeoutMs,
				);
			}),
		]);
		console.log(
			JSON.stringify(
				{
					passed: report.passed,
					cases: report.cases.map(({ id, passed, error }) => ({
						id,
						passed,
						error,
					})),
					output: outputPath,
				},
				null,
				2,
			),
		);
		return report.passed ? 0 : 1;
	} finally {
		if (timer) clearTimeout(timer);
		server.stop(true);
		await stop(desktopChild);
		await stop(frontendChild);
		await nativeLog.close();
		await frontendLog.close();
	}
}

if (import.meta.main) {
	try {
		process.exitCode = await main(process.argv.slice(2));
	} catch (error) {
		console.error(String(error));
		process.exitCode = 2;
	}
}
