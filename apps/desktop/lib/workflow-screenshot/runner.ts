import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type {
	DocScreenshotPlan,
	DocScreenshotResult,
} from "../doc-screenshot/types";
import {
	DOC_SCREENSHOT_PLAN_SCHEMA,
	DOC_SCREENSHOT_RESULT_SCHEMA,
} from "../doc-screenshot/types";
import { inferWorkflowScreenshotFormat } from "./cli";
import { buildWorkflowScreenshotFixture } from "./fixture";
import {
	defaultWorkflowFocus,
	describeWorkflowNodes,
	resolveWorkflowFocus,
	workflowFocusSentinelId,
} from "./focus";
import { enableWorkflowErrorHandling } from "./handle-errors";
import { autoLayoutWorkflowBoard } from "./layout";
import { reconcileFlowScriptForScreenshot } from "./reconcile";
import { type SubprocessStatus, subprocessFailureMessage } from "./subprocess";
import {
	WORKFLOW_NODE_LIST_SCHEMA,
	WORKFLOW_SCREENSHOT_RESULT_SCHEMA,
	type WorkflowFocusTarget,
	type WorkflowNodeDescriptor,
	type WorkflowScreenshotCliOptions,
	type WorkflowScreenshotResult,
} from "./types";

const repositoryRoot = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"../../../..",
);
const screenshotScript = resolve(
	repositoryRoot,
	"apps/desktop/scripts/doc-screenshot.ts",
);
const MAX_SCREENSHOT_OUTPUT_BYTES = 32 * 1024 * 1024;

export interface WorkflowNodeListResult {
	schema: typeof WORKFLOW_NODE_LIST_SCHEMA;
	input: string;
	board: { id: string; name: string };
	nodes: WorkflowNodeDescriptor[];
}

function cssAttributeValue(value: string): string {
	return `"${value
		.replaceAll("\\", "\\\\")
		.replaceAll('"', '\\"')
		.replaceAll("\n", "\\a ")}"`;
}

function captureName(path: string): string {
	const stem = basename(path, extname(path));
	return (
		stem
			.replace(/[^a-zA-Z0-9._-]+/g, "-")
			.replace(/^-+|-+$/g, "")
			.slice(0, 80) || "workflow"
	);
}

async function runScreenshotCli(
	planPath: string,
	options: WorkflowScreenshotCliOptions,
	frontendPort?: number,
): Promise<DocScreenshotResult> {
	const args = [screenshotScript, "--plan", planPath, "--json"];
	if (options.frontendUrl) args.push("--frontend-url", options.frontendUrl);
	if (frontendPort) args.push("--port", String(frontendPort));

	const child = spawn("bun", args, {
		cwd: repositoryRoot,
		stdio: ["ignore", "pipe", "pipe"],
	});
	const stdout: Buffer[] = [];
	const stderr: Buffer[] = [];
	let outputBytes = 0;
	const collect = (chunks: Buffer[], chunk: Buffer) => {
		outputBytes += chunk.byteLength;
		if (outputBytes > MAX_SCREENSHOT_OUTPUT_BYTES) {
			child.kill("SIGTERM");
			return;
		}
		chunks.push(chunk);
	};
	child.stdout.on("data", (chunk: Buffer) => collect(stdout, chunk));
	child.stderr.on("data", (chunk: Buffer) => {
		collect(stderr, chunk);
		if (!options.json) process.stderr.write(chunk);
	});
	const status = await new Promise<SubprocessStatus>((resolveExit, reject) => {
		child.once("error", reject);
		child.once("close", (code, signal) => resolveExit({ code, signal }));
	});
	if (outputBytes > MAX_SCREENSHOT_OUTPUT_BYTES) {
		throw new Error("Screenshot subprocess exceeded its 32 MiB output limit.");
	}

	const stdoutText = Buffer.concat(stdout).toString("utf8").trim();
	const stderrText = Buffer.concat(stderr).toString("utf8").trim();
	let result: DocScreenshotResult | undefined;
	try {
		result = JSON.parse(stdoutText) as DocScreenshotResult;
	} catch {
		// A bounded, human-readable error below is clearer than JSON's parse message.
	}
	if (
		status.code !== 0 ||
		!result ||
		result.schema !== DOC_SCREENSHOT_RESULT_SCHEMA ||
		!result.passed
	) {
		const scenarioError = Array.isArray(result?.scenarios)
			? result.scenarios.find((scenario) => scenario.error)?.error
			: undefined;
		throw new Error(
			subprocessFailureMessage(
				"Workflow screenshot capture",
				status,
				scenarioError,
				stderrText,
			),
		);
	}
	return result;
}

async function availableLoopbackPort(): Promise<number> {
	const server = createServer();
	await new Promise<void>((resolveListen, reject) => {
		server.once("error", reject);
		server.listen(0, "127.0.0.1", resolveListen);
	});
	const address = server.address();
	if (!address || typeof address === "string") {
		server.close();
		throw new Error(
			"Failed to reserve a loopback port for the desktop frontend.",
		);
	}
	await new Promise<void>((resolveClose, reject) => {
		server.close((error) => (error ? reject(error) : resolveClose()));
	});
	return address.port;
}

export function buildWorkflowScreenshotPlan(
	options: WorkflowScreenshotCliOptions,
	output: string,
	focusId?: string,
	focusSentinelId?: string,
): DocScreenshotPlan {
	const format = inferWorkflowScreenshotFormat(output);
	if (!format) throw new Error("Workflow screenshot output format is invalid.");
	const steps: DocScreenshotPlan["scenarios"][number]["steps"] = [
		{
			type: "waitFor",
			selector: ".react-flow__pane",
			state: "visible",
			timeoutMs: options.timeoutMs,
		},
		{
			type: "waitFor",
			selector: ".react-flow__node",
			state: "visible",
			timeoutMs: options.timeoutMs,
		},
	];
	if (focusId) {
		steps.push({
			type: "waitFor",
			selector: `.react-flow__node[data-id=${cssAttributeValue(focusSentinelId ?? focusId)}]`,
			state: "visible",
			timeoutMs: options.timeoutMs,
		});
	} else {
		steps.push({
			type: "click",
			selector: ".react-flow__controls-fitview",
		});
	}
	steps.push(
		{ type: "delay", ms: options.settleMs },
		{
			type: "capture",
			name: captureName(output),
			output: basename(output),
			mode: "viewport",
			format,
			quality: options.quality,
		},
	);

	return {
		schema: DOC_SCREENSHOT_PLAN_SCHEMA,
		app: "desktop",
		outputDir: dirname(output),
		tauriFixture: "workflow.tauri.json",
		defaults: {
			viewport: {
				...options.viewport,
				deviceScaleFactor: options.dpr,
			},
			theme: options.theme,
			format,
			quality: options.quality,
			timeoutMs: options.timeoutMs,
			settleMs: options.settleMs,
			disableAnimations: true,
			hideScrollbars: true,
		},
		scenarios: [
			{
				name: captureName(output),
				path: "/flow",
				query: {
					app: "flowscript-render-app",
					id: "flowscript-render-board",
					...(focusId ? { node: focusId } : {}),
				},
				localStorage: { "tutorial-finished-new": "true" },
				steps,
			},
		],
	};
}

export function resolveWorkflowScreenshotFocus(
	board: IBoard,
	focusNode?: string,
	handleErrorsTarget?: WorkflowFocusTarget,
): WorkflowFocusTarget | undefined {
	if (focusNode) return resolveWorkflowFocus(board, focusNode);
	return handleErrorsTarget ?? defaultWorkflowFocus(board);
}

export async function runWorkflowScreenshot(
	options: WorkflowScreenshotCliOptions,
): Promise<WorkflowScreenshotResult | WorkflowNodeListResult> {
	if (!options.input) throw new Error("A FlowScript input file is required.");
	const renderData = await reconcileFlowScriptForScreenshot(
		options.input,
		options.name,
	);
	if (options.listNodes) {
		return {
			schema: WORKFLOW_NODE_LIST_SCHEMA,
			input: options.input,
			board: { id: renderData.board.id, name: renderData.board.name },
			nodes: describeWorkflowNodes(renderData.board),
		};
	}
	if (!options.output) throw new Error("A screenshot output path is required.");

	const handleErrorsTarget = options.handleErrors
		? enableWorkflowErrorHandling(renderData.board, options.handleErrors)
		: undefined;
	autoLayoutWorkflowBoard(renderData.board, options.layout);
	const focus = resolveWorkflowScreenshotFocus(
		renderData.board,
		options.focusNode,
		handleErrorsTarget,
	);
	const fixture = await buildWorkflowScreenshotFixture(
		renderData.board,
		renderData.catalog,
	);
	const plan = buildWorkflowScreenshotPlan(
		options,
		options.output,
		focus?.id,
		focus ? workflowFocusSentinelId(renderData.board, focus) : undefined,
	);
	const temporaryDirectory = await mkdtemp(
		join(tmpdir(), "flow-like-workflow-screenshot-"),
	);
	const fixturePath = join(temporaryDirectory, "workflow.tauri.json");
	const planPath = join(temporaryDirectory, "workflow.plan.json");
	try {
		await Promise.all([
			writeFile(fixturePath, JSON.stringify(fixture)),
			writeFile(planPath, JSON.stringify(plan)),
		]);
		const frontendPort = options.frontendUrl
			? undefined
			: (options.port ?? (await availableLoopbackPort()));
		const screenshot = await runScreenshotCli(planPath, options, frontendPort);
		return {
			schema: WORKFLOW_SCREENSHOT_RESULT_SCHEMA,
			passed: true,
			input: options.input,
			output: options.output,
			layout: options.layout,
			focus,
			board: {
				id: renderData.board.id,
				name: renderData.board.name,
				nodes: Object.keys(renderData.board.nodes).length,
				layers: Object.keys(renderData.board.layers).length,
			},
			screenshot,
		};
	} finally {
		await rm(temporaryDirectory, { recursive: true, force: true });
	}
}
