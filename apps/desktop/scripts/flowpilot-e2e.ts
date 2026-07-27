#!/usr/bin/env bun

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdir, open, readFile, stat, unlink } from "node:fs/promises";
import { arch, platform, tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
	FLOWPILOT_APP_CREATION_CASES,
	FLOWPILOT_E2E_DEFAULT_MODEL_KEY,
	FLOWPILOT_E2E_MODEL_KEYS,
	type FlowPilotE2ECaseId,
	type FlowPilotE2ECliEnvelope,
	type FlowPilotE2EModelKey,
	buildCasePrompt,
	flowPilotE2ECaseRunTimeoutMs,
	flowPilotE2ECliExitCode,
	flowPilotE2EModel,
	formatAppCreationReport,
	isFlowPilotE2ECliEnvelope,
	normalizeFlowPilotE2ECliEnvelope,
	resolveFlowPilotE2EModelKey,
	resolveFlowPilotE2ERunCases,
} from "../lib/flowpilot-e2e";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(scriptDir, "..");
const MAX_REPEAT = 20;
// Mirrors the runner page's MAX_PARALLEL_CASES / the chat's concurrent-run cap.
const MAX_CONCURRENCY = 4;
// Headroom over each case's own run ceiling for startup, collection and cancellation.
const CASE_OVERHEAD_MS = 5 * 60_000;
const DEFAULT_STARTUP_TIMEOUT_MS = 5 * 60_000;
const CALLBACK_BODY_LIMIT = 256 * 1024 * 1024;

export interface CliOptions {
	caseIds: FlowPilotE2ECaseId[];
	suite?: "smoke" | "full";
	modelKey: FlowPilotE2EModelKey;
	minChars?: number;
	repeat: number;
	concurrency: number;
	failFast: boolean;
	json: boolean;
	list: boolean;
	dryRun: boolean;
	keepDesktop: boolean;
	output?: string;
	timeoutMs?: number;
	frontendUrl?: string;
	config?: string;
	help: boolean;
}

interface CliLock {
	release: () => Promise<void>;
}

interface CliInfrastructureError {
	schema: "flowpilot.app-creation-e2e-cli-error/v1";
	passed: false;
	error: string;
}

export function usage(): string {
	return `FlowPilot app-creation E2E CLI

Usage:
  bun run flowpilot:e2e -- --case simple-agent
  bun run flowpilot:e2e -- --suite smoke --min-chars 1200 --json
  bun run flowpilot:e2e -- --case forum --case ops-dashboard --repeat 3
  bun run flowpilot:e2e -- --case ai-adventure --model sol

Options:
  --case <id>             Select a case; repeat the flag for an ordered subset
  --suite <smoke|full>    Select the three-case smoke suite or all cases (default: smoke)
  --model <${FLOWPILOT_E2E_MODEL_KEYS.join("|")}>    Pin the benchmark model, by alias or model id (default: ${FLOWPILOT_E2E_DEFAULT_MODEL_KEY})
  --min-chars <n>         Override the non-whitespace FlowScript sanity floor
  --repeat <n>            Repeat each selected case, 1-${MAX_REPEAT} (default: 1)
  --concurrency <n>       Cases in flight at once, 1-${MAX_CONCURRENCY} (default: 1; not with --fail-fast)
  --fail-fast             Stop after the first failed case
  --output <path>         Copy the complete JSON envelope to this path
  --json                  Print the complete JSON envelope to stdout
  --timeout-ms <n>        Overall startup + generation timeout
  --frontend-url <url>    Reuse an existing loopback Next dev server
  --config <path>         Override the platform Tauri config
  --keep-desktop          Leave the spawned desktop/dev server running after the result
  --list                  List cases without starting Tauri
  --dry-run               Print resolved prompts without starting Tauri
  --help                  Show this help

The command launches the real development Tauri app and its existing GlobalToolBridge. Child
process logs go to stderr; --json reserves stdout for one machine-readable result.`;
}

function valueAfter(
	args: string[],
	index: number,
	flag: string,
): [string, number] {
	const current = args[index] ?? "";
	if (current.startsWith(`${flag}=`)) {
		return [current.slice(flag.length + 1), index];
	}
	const next = args[index + 1];
	if (!next || next.startsWith("--")) {
		throw new Error(`${flag} requires a value.`);
	}
	return [next, index + 1];
}

function positiveInteger(value: string, flag: string): number {
	const parsed = Number(value);
	if (!Number.isSafeInteger(parsed) || parsed < 1) {
		throw new Error(`${flag} must be a positive integer.`);
	}
	return parsed;
}

export function parseArgs(args: string[]): CliOptions {
	const options: CliOptions = {
		caseIds: [],
		modelKey: FLOWPILOT_E2E_DEFAULT_MODEL_KEY,
		repeat: 1,
		concurrency: 1,
		failFast: false,
		json: false,
		list: false,
		dryRun: false,
		keepDesktop: false,
		help: false,
	};
	const knownCases = new Set(
		FLOWPILOT_APP_CREATION_CASES.map((caseDefinition) => caseDefinition.id),
	);
	const normalizedArgs = args.filter((arg) => arg !== "--");

	for (let index = 0; index < normalizedArgs.length; index += 1) {
		const arg = normalizedArgs[index] ?? "";
		if (arg === "--help" || arg === "-h") {
			options.help = true;
		} else if (arg === "--json") {
			options.json = true;
		} else if (arg === "--list") {
			options.list = true;
		} else if (arg === "--dry-run") {
			options.dryRun = true;
		} else if (arg === "--fail-fast") {
			options.failFast = true;
		} else if (arg === "--keep-desktop") {
			options.keepDesktop = true;
		} else if (arg === "--case" || arg.startsWith("--case=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--case");
			index = consumed;
			if (!knownCases.has(value as FlowPilotE2ECaseId)) {
				throw new Error(`Unknown FlowPilot E2E case: ${value}`);
			}
			options.caseIds.push(value as FlowPilotE2ECaseId);
		} else if (arg === "--suite" || arg.startsWith("--suite=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--suite");
			index = consumed;
			if (value !== "smoke" && value !== "full") {
				throw new Error("--suite must be smoke or full.");
			}
			options.suite = value;
		} else if (arg === "--model" || arg.startsWith("--model=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--model");
			index = consumed;
			options.modelKey = resolveFlowPilotE2EModelKey(value);
		} else if (arg === "--min-chars" || arg.startsWith("--min-chars=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--min-chars",
			);
			index = consumed;
			options.minChars = positiveInteger(value, "--min-chars");
		} else if (arg === "--repeat" || arg.startsWith("--repeat=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--repeat");
			index = consumed;
			options.repeat = positiveInteger(value, "--repeat");
			if (options.repeat > MAX_REPEAT) {
				throw new Error(`--repeat cannot exceed ${MAX_REPEAT}.`);
			}
		} else if (arg === "--concurrency" || arg.startsWith("--concurrency=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--concurrency",
			);
			index = consumed;
			options.concurrency = positiveInteger(value, "--concurrency");
			if (options.concurrency > MAX_CONCURRENCY) {
				throw new Error(`--concurrency cannot exceed ${MAX_CONCURRENCY}.`);
			}
		} else if (arg === "--timeout-ms" || arg.startsWith("--timeout-ms=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--timeout-ms",
			);
			index = consumed;
			options.timeoutMs = positiveInteger(value, "--timeout-ms");
		} else if (arg === "--output" || arg.startsWith("--output=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--output");
			index = consumed;
			options.output = resolve(process.cwd(), value);
		} else if (arg === "--frontend-url" || arg.startsWith("--frontend-url=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--frontend-url",
			);
			index = consumed;
			options.frontendUrl = value;
		} else if (arg === "--config" || arg.startsWith("--config=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--config");
			index = consumed;
			options.config = resolve(process.cwd(), value);
		} else {
			throw new Error(`Unknown argument: ${arg}`);
		}
	}

	if (options.caseIds.length > 0 && options.suite) {
		throw new Error("Use either --case or --suite, not both.");
	}
	if (options.concurrency > 1 && options.failFast) {
		throw new Error(
			"--fail-fast needs sequential cases; drop it or use --concurrency 1.",
		);
	}
	options.caseIds = [...new Set(options.caseIds)];
	return options;
}

function platformConfig(): string {
	const os = platform();
	const cpu = arch();
	if (os === "darwin") {
		return resolve(
			desktopDir,
			cpu === "arm64"
				? "src-tauri/configs/tauri.macos.arm.conf.json"
				: "src-tauri/configs/tauri.macos.intel.conf.json",
		);
	}
	if (os === "win32") {
		return resolve(
			desktopDir,
			cpu === "arm64"
				? "src-tauri/configs/tauri.win.arm.conf.json"
				: "src-tauri/configs/tauri.win.x64.conf.json",
		);
	}
	if (os === "linux" && cpu === "x64") {
		return resolve(desktopDir, "src-tauri/configs/tauri.linux.x64.conf.json");
	}
	throw new Error(`Unsupported FlowPilot desktop target: ${os}/${cpu}.`);
}

function validatedFrontendUrl(value: string): URL {
	const url = new URL(value);
	if (
		url.protocol !== "http:" ||
		(url.hostname !== "localhost" && url.hostname !== "127.0.0.1")
	) {
		throw new Error("--frontend-url must be an http loopback URL.");
	}
	// Tauri remote capabilities and macOS ATS are scoped to `localhost`.
	if (url.hostname === "127.0.0.1") url.hostname = "localhost";
	return url;
}

function runningDesktopProcessIds(): string[] {
	if (platform() === "win32") {
		const result = spawnSync("tasklist", ["/FO", "CSV", "/NH"], {
			encoding: "utf8",
			windowsHide: true,
		});
		if (
			result.error ||
			result.status !== 0 ||
			typeof result.stdout !== "string"
		) {
			throw new Error(
				`Could not inspect running Windows processes: ${result.error?.message ?? `tasklist exited ${result.status}`}.`,
			);
		}
		return result.stdout
			.split(/\r?\n/)
			.map((line) => line.match(/^"([^"]+)","(\d+)"/))
			.filter((match): match is RegExpMatchArray => {
				const image = match?.[1]?.toLowerCase();
				return image === "flow-like-desktop.exe" || image === "flow like.exe";
			})
			.map((match) => match[2])
			.filter((pid): pid is string => Boolean(pid));
	}
	const result = spawnSync("ps", ["-axo", "pid=,command="], {
		encoding: "utf8",
	});
	if (
		result.error ||
		result.status !== 0 ||
		typeof result.stdout !== "string"
	) {
		throw new Error(
			`Could not inspect running desktop processes: ${result.error?.message ?? `ps exited ${result.status}`}.`,
		);
	}
	return result.stdout
		.split(/\r?\n/)
		.map((line) => line.match(/^\s*(\d+)\s+(.+)$/))
		.filter((match): match is RegExpMatchArray =>
			Boolean(
				match && /(?:^|\/)flow-like-desktop(?:\s|$)/.test(match[2] ?? ""),
			),
		)
		.map((match) => match[1] ?? "")
		.filter(Boolean);
}

function assertNoRunningDesktop(): void {
	const pids = runningDesktopProcessIds();
	if (pids.length === 0) return;
	throw new Error(
		`Flow Like desktop is already running (pid ${pids.join(", ")}). Close it before running the E2E CLI so local app data has one writer.`,
	);
}

function processIsAlive(pid: number): boolean {
	try {
		process.kill(pid, 0);
		return true;
	} catch (error) {
		return (
			error instanceof Error &&
			"code" in error &&
			(error as NodeJS.ErrnoException).code === "EPERM"
		);
	}
}

async function acquireCliLock(runId: string): Promise<CliLock> {
	const lockPath = resolve(tmpdir(), "flow-like-flowpilot-e2e", "runner.lock");
	await mkdir(dirname(lockPath), { recursive: true });
	for (let attempt = 0; attempt < 2; attempt += 1) {
		try {
			const handle = await open(lockPath, "wx");
			try {
				await handle.writeFile(
					JSON.stringify({
						pid: process.pid,
						runId,
						startedAt: new Date().toISOString(),
					}),
				);
			} finally {
				await handle.close();
			}
			return {
				release: async () => {
					try {
						const current = JSON.parse(await readFile(lockPath, "utf8")) as {
							runId?: unknown;
						};
						if (current.runId === runId) await unlink(lockPath);
					} catch (error) {
						if (
							!(
								error instanceof Error &&
								"code" in error &&
								(error as NodeJS.ErrnoException).code === "ENOENT"
							)
						) {
							console.error(
								`Could not release FlowPilot E2E lock: ${String(error)}`,
							);
						}
					}
				},
			};
		} catch (error) {
			const code =
				error instanceof Error && "code" in error
					? (error as NodeJS.ErrnoException).code
					: undefined;
			if (code !== "EEXIST") throw error;
			let owner: { pid?: unknown; runId?: unknown } = {};
			let observed = "";
			try {
				observed = await readFile(lockPath, "utf8");
				owner = JSON.parse(observed) as typeof owner;
			} catch {
				let ageMs = 0;
				try {
					ageMs = Date.now() - (await stat(lockPath)).mtimeMs;
				} catch {
					continue;
				}
				if (ageMs < 60_000) {
					throw new Error(
						"Another FlowPilot E2E CLI is acquiring the runner lock.",
					);
				}
			}
			if (typeof owner.pid === "number" && processIsAlive(owner.pid)) {
				throw new Error(
					`Another FlowPilot E2E CLI is running (pid ${owner.pid}, run ${String(owner.runId ?? "unknown")}).`,
				);
			}
			try {
				if ((await readFile(lockPath, "utf8")) !== observed) continue;
				await unlink(lockPath);
			} catch (unlinkError) {
				const unlinkCode =
					unlinkError instanceof Error && "code" in unlinkError
						? (unlinkError as NodeJS.ErrnoException).code
						: undefined;
				if (unlinkCode !== "ENOENT") throw unlinkError;
			}
		}
	}
	throw new Error("Could not acquire the FlowPilot E2E CLI lock.");
}

function prepareWindowsPrerequisites(): void {
	if (platform() !== "win32") return;
	const result = spawnSync(
		"bun",
		[
			"./scripts/prepare-windows-prereqs.ts",
			"--arch",
			arch() === "arm64" ? "arm64" : "x64",
		],
		{ cwd: desktopDir, stdio: ["ignore", 2, 2] },
	);
	if (result.error || result.status !== 0) {
		throw new Error(
			`Windows prerequisite preparation failed: ${result.error?.message ?? `exit ${result.status}`}`,
		);
	}
}

async function waitForChildExit(
	child: ChildProcess,
	timeoutMs: number,
): Promise<void> {
	if (child.exitCode !== null || child.signalCode !== null) return;
	await new Promise<void>((resolveWait) => {
		const finish = () => {
			clearTimeout(timeout);
			child.off("exit", finish);
			resolveWait();
		};
		const timeout = setTimeout(finish, timeoutMs);
		child.once("exit", finish);
		if (child.exitCode !== null || child.signalCode !== null) finish();
	});
}

function processGroupIsAlive(pid: number): boolean {
	try {
		process.kill(-pid, 0);
		return true;
	} catch (error) {
		const code =
			error instanceof Error && "code" in error
				? (error as NodeJS.ErrnoException).code
				: undefined;
		if (code === "ESRCH") return false;
		if (code !== "EPERM") {
			console.error(
				`Could not probe Tauri process group ${pid}: ${String(error)}`,
			);
		}
		return true;
	}
}

async function waitForProcessGroupExit(
	pid: number,
	timeoutMs: number,
): Promise<boolean> {
	const deadline = Date.now() + timeoutMs;
	while (processGroupIsAlive(pid) && Date.now() < deadline) {
		await new Promise<void>((resolveWait) => setTimeout(resolveWait, 50));
	}
	return !processGroupIsAlive(pid);
}

function signalProcessGroup(pid: number, signal: NodeJS.Signals): boolean {
	try {
		process.kill(-pid, signal);
		return true;
	} catch (error) {
		const code =
			error instanceof Error && "code" in error
				? (error as NodeJS.ErrnoException).code
				: undefined;
		if (code === "ESRCH") return false;
		console.error(
			`Could not send ${signal} to Tauri process group ${pid}: ${String(error)}`,
		);
		return false;
	}
}

async function stopChild(child: ChildProcess): Promise<void> {
	if (platform() === "win32") {
		if (child.pid) {
			spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
				stdio: "ignore",
			});
		} else {
			child.kill("SIGTERM");
		}
		await waitForChildExit(child, 2_000);
		return;
	}
	if (!child.pid) {
		if (child.exitCode === null && child.signalCode === null) {
			child.kill("SIGTERM");
			await waitForChildExit(child, 10_000);
			if (child.exitCode === null && child.signalCode === null) {
				child.kill("SIGKILL");
				await waitForChildExit(child, 2_000);
			}
		}
		return;
	}

	const pid = child.pid;
	if (!signalProcessGroup(pid, "SIGTERM")) {
		if (child.exitCode === null && child.signalCode === null) {
			child.kill("SIGTERM");
			await waitForChildExit(child, 10_000);
		}
		return;
	}
	if (!(await waitForProcessGroupExit(pid, 10_000))) {
		signalProcessGroup(pid, "SIGKILL");
		if (!(await waitForProcessGroupExit(pid, 2_000))) {
			console.error(`Tauri process group ${pid} survived SIGKILL.`);
		}
	}
}

function printCaseList(json: boolean): void {
	const cases = FLOWPILOT_APP_CREATION_CASES.map((caseDefinition) => ({
		id: caseDefinition.id,
		title: caseDefinition.title,
		smoke: caseDefinition.smoke,
		minChars: caseDefinition.requirements.minFlowScriptNonWhitespaceChars,
		maxChars: caseDefinition.requirements.maxFlowScriptNonWhitespaceChars,
	}));
	if (json) console.log(JSON.stringify(cases, null, 2));
	else {
		for (const item of cases) {
			console.log(
				`${item.id.padEnd(18)} ${item.smoke ? "smoke" : "full "} ${item.minChars}-${item.maxChars}  ${item.title}`,
			);
		}
	}
}

function printDryRun(options: CliOptions): void {
	const definitions = resolveFlowPilotE2ERunCases({
		caseIds: options.caseIds.length ? options.caseIds : undefined,
		suite: options.caseIds.length ? undefined : options.suite,
	});
	const prompts = definitions.map((caseDefinition) =>
		buildCasePrompt(caseDefinition, "[CLI DRY RUN]", {
			minFlowScriptNonWhitespaceChars: options.minChars,
		}),
	);
	const value = {
		modelKey: options.modelKey,
		model: flowPilotE2EModel(options.modelKey),
		repeat: options.repeat,
		failFast: options.failFast,
		prompts,
	};
	if (options.json) console.log(JSON.stringify(value, null, 2));
	else {
		for (const prompt of prompts) {
			console.log(
				`\n### ${prompt.caseDefinition.id}: ${prompt.expectedAppName}\n`,
			);
			console.log(prompt.prompt);
		}
	}
}

function printEnvelope(
	envelope: FlowPilotE2ECliEnvelope,
	output: string,
	json: boolean,
): void {
	if (json) {
		console.log(JSON.stringify(envelope));
		return;
	}
	console.log(
		`${envelope.passed ? "PASS" : "FAIL"} FlowPilot E2E (${envelope.selection.modelKey}): ${envelope.summary.passed}/${envelope.summary.requestedRuns} runs passed in ${Math.round(envelope.durationMs / 1000)}s`,
	);
	if (envelope.error) console.log(`Infrastructure: ${envelope.error}`);
	for (const artifact of envelope.artifacts) {
		if (artifact.report)
			console.log(`\n${formatAppCreationReport(artifact.report)}`);
		else {
			console.log(
				`\nFAIL ${artifact.caseId}: ${artifact.error ?? "case produced no validation report"}`,
			);
		}
		if (artifact.failureFingerprint) {
			console.log(`fingerprint=${artifact.failureFingerprint}`);
		}
	}
	console.log(`\nArtifact: ${output}`);
}

async function run(options: CliOptions): Promise<number> {
	const definitions = resolveFlowPilotE2ERunCases({
		caseIds: options.caseIds.length ? options.caseIds : undefined,
		suite: options.caseIds.length ? undefined : options.suite,
	});
	for (const caseDefinition of definitions) {
		buildCasePrompt(caseDefinition, "[CLI PREFLIGHT]", {
			minFlowScriptNonWhitespaceChars: options.minChars,
		});
	}

	const runId = `e2e_${Date.now()}_${randomBytes(8).toString("hex")}`;
	const cliLock = await acquireCliLock(runId);
	try {
		assertNoRunningDesktop();
		prepareWindowsPrerequisites();
		const nonce = randomBytes(24).toString("hex");
		let callbackResolve!: (value: FlowPilotE2ECliEnvelope) => void;
		let callbackReject!: (reason: Error) => void;
		let callbackSettled = false;
		const callbackPromise = new Promise<FlowPilotE2ECliEnvelope>(
			(resolveResult, rejectResult) => {
				callbackResolve = resolveResult;
				callbackReject = rejectResult;
			},
		);
		const settleCallbackError = (error: Error) => {
			if (callbackSettled) return;
			callbackSettled = true;
			// Let the HTTP 400 reach the webview before the controller stops the server.
			setTimeout(() => callbackReject(error), 100);
		};

		const frontendBase = options.frontendUrl
			? validatedFrontendUrl(options.frontendUrl)
			: new URL("http://localhost:3000");
		const runnerUrl = new URL("/developer/flowpilot-e2e", frontendBase);
		const callbackExpectation = {
			runId,
			caseIds: definitions.map((item) => item.id),
			modelKey: options.modelKey,
			repeat: options.repeat,
			minFlowScriptNonWhitespaceChars: options.minChars,
			failFast: options.failFast,
		};
		const callbackServer = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			maxRequestBodySize: CALLBACK_BODY_LIMIT,
			async fetch(request) {
				const url = new URL(request.url);
				const corsHeaders = {
					"access-control-allow-origin": runnerUrl.origin,
					"access-control-allow-methods": "POST, OPTIONS",
					"access-control-allow-headers": "content-type",
					vary: "Origin",
				};
				if (url.pathname !== `/${nonce}`)
					return new Response("Not found", { status: 404 });
				const origin = request.headers.get("origin");
				if (origin && origin !== runnerUrl.origin) {
					return new Response("Forbidden", { status: 403 });
				}
				if (request.method === "OPTIONS") {
					return new Response(null, { status: 204, headers: corsHeaders });
				}
				if (request.method !== "POST") {
					return new Response("Method not allowed", {
						status: 405,
						headers: corsHeaders,
					});
				}
				try {
					const body: unknown = await request.json();
					if (!isFlowPilotE2ECliEnvelope(body, runId)) {
						settleCallbackError(
							new Error("Invalid FlowPilot E2E result envelope."),
						);
						return Response.json(
							{ error: "Invalid FlowPilot E2E result envelope." },
							{ status: 400, headers: corsHeaders },
						);
					}
					const normalized = normalizeFlowPilotE2ECliEnvelope(
						body,
						callbackExpectation,
					);
					if (!callbackSettled) {
						callbackSettled = true;
						setTimeout(() => callbackResolve(normalized), 100);
					}
					return Response.json(
						{ status: "ok", runId },
						{ headers: corsHeaders },
					);
				} catch (error) {
					const callbackError =
						error instanceof Error ? error : new Error(String(error));
					settleCallbackError(callbackError);
					return Response.json(
						{ error: callbackError.message },
						{ status: 400, headers: corsHeaders },
					);
				}
			},
		});

		const callbackUrl = `http://localhost:${callbackServer.port}/${nonce}`;
		runnerUrl.searchParams.set("cli", "1");
		runnerUrl.searchParams.set("cliRunId", runId);
		runnerUrl.searchParams.set("callback", callbackUrl);
		runnerUrl.searchParams.set(
			"cases",
			definitions.map((item) => item.id).join(","),
		);
		runnerUrl.searchParams.set("model", options.modelKey);
		runnerUrl.searchParams.set("repeat", String(options.repeat));
		runnerUrl.searchParams.set("concurrency", String(options.concurrency));
		if (options.minChars !== undefined) {
			runnerUrl.searchParams.set("minChars", String(options.minChars));
		}
		if (options.failFast) runnerUrl.searchParams.set("failFast", "1");

		const inlineConfig = options.frontendUrl
			? JSON.stringify({
					build: {
						beforeDevCommand: null,
						devUrl: frontendBase.origin,
					},
				})
			: undefined;
		const config = options.config ?? platformConfig();
		const env: NodeJS.ProcessEnv = {
			...process.env,
			FLOWPILOT_E2E_CLI_RUN_ID: runId,
			FLOWPILOT_E2E_CLI_URL: runnerUrl.toString(),
			NEXT_TELEMETRY_DISABLED: "1",
		};
		// A user-level sccache wrapper cannot open its daemon from Codex's sandbox and
		// otherwise emits one fallback error per rustc process during the feedback loop.
		if (process.env.CODEX_SANDBOX) env.RUSTC_WRAPPER = "";
		if (platform() === "darwin") {
			env.ORT_LIB_LOCATION ??= resolve(
				desktopDir,
				"src-tauri/gen/apple/thirdparty/onnxruntime.xcframework/macos-arm64_x86_64",
			);
			env.MACOSX_DEPLOYMENT_TARGET ??= "14.0";
		}

		const pinnedModel = flowPilotE2EModel(options.modelKey);
		console.error(
			`Starting ${definitions.length * options.repeat} FlowPilot run(s) with ${pinnedModel.provider}/${pinnedModel.model}/${pinnedModel.reasoningEffort}...`,
		);
		const tauriArgs = ["run", "tauri", "dev", "--no-watch", "--config", config];
		if (inlineConfig) tauriArgs.push("--config", inlineConfig);
		// The marker reaches an already-running instance and prevents its window being focused.
		tauriArgs.push("--", "--", "--flowpilot-e2e-cli");
		const child = spawn("bun", tauriArgs, {
			cwd: desktopDir,
			env,
			stdio: ["ignore", 2, 2],
			detached: platform() !== "win32",
		});
		let stoppingForSignal = false;
		const stopForSignal = (exitCode: number) => {
			if (stoppingForSignal) return;
			stoppingForSignal = true;
			callbackServer.stop(true);
			void stopChild(child).finally(async () => {
				await cliLock.release();
				process.exit(exitCode);
			});
		};
		const onSigint = () => stopForSignal(130);
		const onSigterm = () => stopForSignal(143);
		process.once("SIGINT", onSigint);
		process.once("SIGTERM", onSigterm);

		const timeoutMs =
			options.timeoutMs ??
			DEFAULT_STARTUP_TIMEOUT_MS +
				options.repeat *
					definitions.reduce(
						(total, caseDefinition) =>
							total +
							flowPilotE2ECaseRunTimeoutMs(caseDefinition) +
							CASE_OVERHEAD_MS,
						0,
					);
		let timeout: ReturnType<typeof setTimeout> | undefined;
		const timeoutPromise = new Promise<FlowPilotE2ECliEnvelope>(
			(_resolve, reject) => {
				timeout = setTimeout(
					() =>
						reject(new Error(`FlowPilot E2E timed out after ${timeoutMs}ms.`)),
					timeoutMs,
				);
			},
		);
		const childExitPromise = new Promise<FlowPilotE2ECliEnvelope>(
			(_resolve, reject) => {
				child.once("error", (error) => reject(error));
				child.once("exit", (code, signal) => {
					setTimeout(() => {
						if (!callbackSettled) {
							const collisionHint =
								code === 0 && signal === null
									? " Another Flow Like desktop instance is likely already open; close it before running this CLI."
									: "";
							reject(
								new Error(
									`Tauri exited before returning an E2E artifact (${signal ?? `code ${code}`}).${collisionHint}`,
								),
							);
						}
					}, 250);
				});
			},
		);

		try {
			const envelope = await Promise.race([
				callbackPromise,
				timeoutPromise,
				childExitPromise,
			]);
			if (timeout) clearTimeout(timeout);
			const output =
				options.output ??
				resolve(
					tmpdir(),
					"flow-like-flowpilot-e2e",
					"artifacts",
					`${runId}.json`,
				);
			await mkdir(dirname(output), { recursive: true });
			await Bun.write(output, `${JSON.stringify(envelope, null, 2)}\n`);
			printEnvelope(envelope, output, options.json);

			if (options.keepDesktop) {
				child.unref();
			} else {
				await stopChild(child);
			}
			return flowPilotE2ECliExitCode(envelope);
		} catch (error) {
			if (timeout) clearTimeout(timeout);
			await stopChild(child);
			throw error;
		} finally {
			process.off("SIGINT", onSigint);
			process.off("SIGTERM", onSigterm);
			callbackServer.stop(true);
		}
	} finally {
		await cliLock.release();
	}
}

export async function main(args = process.argv.slice(2)): Promise<void> {
	const jsonRequested = args.includes("--json");
	try {
		const options = parseArgs(args);
		if (options.help) {
			console.log(usage());
			return;
		}
		if (options.list) {
			printCaseList(options.json);
			return;
		}
		if (options.dryRun) {
			printDryRun(options);
			return;
		}
		process.exitCode = await run(options);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		if (jsonRequested) {
			const result: CliInfrastructureError = {
				schema: "flowpilot.app-creation-e2e-cli-error/v1",
				passed: false,
				error: message,
			};
			console.log(JSON.stringify(result));
		}
		console.error(`FlowPilot E2E CLI error: ${message}`);
		process.exitCode = 2;
	}
}

if (import.meta.main) await main();
