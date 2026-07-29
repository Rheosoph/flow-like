#!/usr/bin/env bun

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { arch, platform } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
	type DocScreenshotCliOptions,
	directPlanFromOptions,
	parseDocScreenshotArgs,
} from "../lib/doc-screenshot/cli";
import {
	loadDocScreenshotPlan,
	loadDocScreenshotTauriFixture,
} from "../lib/doc-screenshot/plan";
import { runDocScreenshotPlan } from "../lib/doc-screenshot/runner";
import type {
	DocScreenshotApp,
	DocScreenshotPlan,
	DocScreenshotResult,
} from "../lib/doc-screenshot/types";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(scriptDir, "..");
const repoDir = resolve(desktopDir, "../..");

interface SpawnedServer {
	child: ChildProcess;
	url: URL;
}

export function usage(): string {
	return `Flow-Like documentation screenshot CLI

Usage:
  bun run docs:screenshot -- --app web --path /onboarding --output /tmp/onboarding.webp
  bun run docs:screenshot -- --plan apps/desktop/lib/doc-screenshot/examples/onboarding.plan.json
  bun run docs:screenshot -- --frontend-url http://127.0.0.1:3000 --path '/settings/ai?tab=models'

Direct capture:
  --app <desktop|web>     Frontend to start (default: desktop)
  --path <path>           Same-origin page path, optionally including a query string
  --query <key=value>     Add a query parameter; repeat for arrays
  --output <path>         .webp, .png, .jpg, or .jpeg output path
  --viewport <WxH>        CSS viewport (default: 1624x1060)
  --dpr <number>          Device scale factor, 0.5-4 (default: 2)
  --theme <light|dark>    Color scheme (default: light)
  --format <format>       webp (lossless), png, or jpeg
  --quality <1-100>       JPEG quality
  --wait-for <selector>   Wait for a visible selector before capture
  --full-page             Capture the full document
  --selector <selector>   Capture one element instead of the viewport

Interaction plan:
  --plan <path>           Run a validated JSON plan with navigation and interaction steps

Server and output:
  --frontend-url <url>    Reuse an existing loopback frontend instead of starting one
  --port <number>         Port for the spawned frontend (desktop 3000, web 3001)
  --output-dir <path>     Override a plan's output directory
  --timeout-ms <number>   Direct-mode navigation/action timeout
  --settle-ms <number>    Direct-mode visual settle delay
  --keep-server           Leave a frontend spawned by this command running
  --json                  Reserve stdout for the result envelope
  --help                  Show this help

Plans support goto, click, drag, fill, type, press, select, check, hover, scroll,
waitFor, delay, and capture steps. See apps/desktop/lib/doc-screenshot/README.md.`;
}

function validatedLoopbackUrl(value: string): URL {
	const url = new URL(value);
	if (
		url.protocol !== "http:" ||
		(url.hostname !== "localhost" &&
			url.hostname !== "127.0.0.1" &&
			url.hostname !== "[::1]")
	) {
		throw new Error("Frontend URL must be an http loopback URL.");
	}
	url.pathname = url.pathname.replace(/\/+$/, "") || "/";
	url.search = "";
	url.hash = "";
	return url;
}

function defaultPort(app: DocScreenshotApp): number {
	return app === "desktop" ? 3000 : 3001;
}

function appDirectory(app: DocScreenshotApp): string {
	return resolve(repoDir, "apps", app);
}

function serverProcessIsRunning(child: ChildProcess): boolean {
	return child.exitCode === null && child.signalCode === null;
}

async function waitForServer(
	server: SpawnedServer,
	timeoutMs: number,
): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	let lastError = "not ready";
	while (Date.now() < deadline) {
		if (!serverProcessIsRunning(server.child)) {
			throw new Error(
				`Frontend exited before becoming ready (exit ${server.child.exitCode}, signal ${server.child.signalCode}).`,
			);
		}
		try {
			const response = await fetch(server.url, {
				redirect: "manual",
				signal: AbortSignal.timeout(2_000),
			});
			if (response.status >= 100) return;
		} catch (error) {
			lastError = error instanceof Error ? error.message : String(error);
		}
		await new Promise<void>((resolveWait) => setTimeout(resolveWait, 250));
	}
	throw new Error(
		`Frontend did not become ready at ${server.url.origin}: ${lastError}`,
	);
}

function spawnFrontend(app: DocScreenshotApp, port: number): SpawnedServer {
	const nextBin = resolve(repoDir, "node_modules/next/dist/bin/next");
	const args = [
		nextBin,
		"dev",
		"--hostname",
		"127.0.0.1",
		"--port",
		String(port),
	];
	if (app === "desktop") args.push("--turbopack");
	console.error(`Starting ${app} frontend on http://127.0.0.1:${port}...`);
	const child = spawn("bun", args, {
		cwd: appDirectory(app),
		env: {
			...process.env,
			NEXT_TELEMETRY_DISABLED: "1",
		},
		stdio: ["ignore", 2, 2],
		detached: platform() !== "win32",
	});
	return {
		child,
		url: new URL(`http://127.0.0.1:${port}`),
	};
}

async function waitForChildExit(
	child: ChildProcess,
	timeoutMs: number,
): Promise<void> {
	if (!serverProcessIsRunning(child)) return;
	await new Promise<void>((resolveWait) => {
		const finish = () => {
			clearTimeout(timeout);
			child.off("exit", finish);
			resolveWait();
		};
		const timeout = setTimeout(finish, timeoutMs);
		child.once("exit", finish);
		if (!serverProcessIsRunning(child)) finish();
	});
}

async function stopFrontend(child: ChildProcess): Promise<void> {
	if (!serverProcessIsRunning(child)) return;
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
		child.kill("SIGTERM");
		await waitForChildExit(child, 5_000);
		return;
	}
	try {
		process.kill(-child.pid, "SIGTERM");
	} catch {
		child.kill("SIGTERM");
	}
	await waitForChildExit(child, 10_000);
	if (serverProcessIsRunning(child)) {
		try {
			process.kill(-child.pid, "SIGKILL");
		} catch {
			child.kill("SIGKILL");
		}
		await waitForChildExit(child, 2_000);
	}
}

function printResult(result: DocScreenshotResult, json: boolean): void {
	if (json) {
		console.log(JSON.stringify(result));
		return;
	}
	console.log(
		`${result.passed ? "PASS" : "FAIL"} documentation screenshots: ${result.summary.screenshots} image(s), ${result.summary.scenariosPassed}/${result.summary.scenarios} scenario(s) passed in ${(result.durationMs / 1000).toFixed(1)}s`,
	);
	for (const scenario of result.scenarios) {
		console.log(`\n${scenario.passed ? "PASS" : "FAIL"} ${scenario.name}`);
		if (scenario.error) console.log(`  ${scenario.error}`);
		for (const artifact of scenario.artifacts) {
			console.log(
				`  ${artifact.pixels.width}x${artifact.pixels.height} ${artifact.mimeType} ${artifact.path}`,
			);
		}
	}
}

async function resolvePlan(options: DocScreenshotCliOptions): Promise<{
	plan: DocScreenshotPlan;
	planPath?: string;
}> {
	if (!options.plan) return { plan: directPlanFromOptions(options) };
	return {
		plan: await loadDocScreenshotPlan(options.plan),
		planPath: options.plan,
	};
}

async function run(options: DocScreenshotCliOptions): Promise<number> {
	const { plan, planPath } = await resolvePlan(options);
	const app = plan.app;
	const explicitFrontend = options.frontendUrl ?? plan.baseUrl;
	let spawned: SpawnedServer | undefined;
	let frontendUrl: URL;
	if (explicitFrontend) {
		frontendUrl = validatedLoopbackUrl(explicitFrontend);
	} else {
		spawned = spawnFrontend(app, options.port ?? defaultPort(app));
		frontendUrl = spawned.url;
	}
	const outputDir = resolve(options.outputDir ?? plan.outputDir);
	const fixturePath =
		plan.tauriFixture && planPath
			? resolve(dirname(planPath), plan.tauriFixture)
			: plan.tauriFixture
				? resolve(process.cwd(), plan.tauriFixture)
				: undefined;
	const fixture = fixturePath
		? await loadDocScreenshotTauriFixture(fixturePath)
		: undefined;
	let stopping = false;
	const stopForSignal = (exitCode: number) => {
		if (stopping) return;
		stopping = true;
		if (!spawned) {
			process.exit(exitCode);
			return;
		}
		void stopFrontend(spawned.child).finally(() => process.exit(exitCode));
	};
	const onSigint = () => stopForSignal(130);
	const onSigterm = () => stopForSignal(143);
	process.once("SIGINT", onSigint);
	process.once("SIGTERM", onSigterm);
	try {
		if (spawned) {
			await waitForServer(spawned, Math.max(plan.defaults.timeoutMs, 60_000));
		}
		console.error(
			`Capturing ${plan.scenarios.length} documentation scenario(s)...`,
		);
		const result = await runDocScreenshotPlan(plan, {
			baseUrl: frontendUrl.origin,
			outputDir,
			tauriFixture: fixture,
		});
		printResult(result, options.json);
		return result.passed ? 0 : 1;
	} finally {
		process.off("SIGINT", onSigint);
		process.off("SIGTERM", onSigterm);
		if (spawned && !options.keepServer) await stopFrontend(spawned.child);
	}
}

export async function main(args = Bun.argv.slice(2)): Promise<number> {
	let options: DocScreenshotCliOptions;
	try {
		options = parseDocScreenshotArgs(args);
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
					schema: "flow-like.doc-screenshot-cli-error/v1",
					passed: false,
					error: message,
					platform: `${platform()}/${arch()}`,
				}),
			);
		} else {
			console.error(`Documentation screenshot failed: ${message}`);
		}
		return 2;
	}
}

if (import.meta.main) {
	process.exitCode = await main();
}
