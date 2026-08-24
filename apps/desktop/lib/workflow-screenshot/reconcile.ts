import { spawn } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { WORKFLOW_SCREENSHOT_BOARD_ID } from "./fixture";
import { type SubprocessStatus, subprocessFailureMessage } from "./subprocess";
import { WORKFLOW_RENDER_DATA_SCHEMA, type WorkflowRenderData } from "./types";

const repositoryRoot = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"../../../..",
);
const MAX_HELPER_OUTPUT_BYTES = 128 * 1024 * 1024;

interface HelperErrorEnvelope {
	error?: string;
	diagnostics?: string[];
}

function parseJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

export class WorkflowReconcileError extends Error {
	constructor(
		message: string,
		readonly diagnostics: string[] = [],
	) {
		super(message);
		this.name = "WorkflowReconcileError";
	}
}

export async function reconcileFlowScriptForScreenshot(
	input: string,
	name?: string,
): Promise<WorkflowRenderData> {
	const args = [
		"run",
		"--quiet",
		"-p",
		"flowscript-render-data",
		"--",
		input,
		"--board-id",
		WORKFLOW_SCREENSHOT_BOARD_ID,
	];
	if (name) args.push("--name", name);

	const child = spawn("cargo", args, {
		cwd: repositoryRoot,
		stdio: ["ignore", "pipe", "pipe"],
	});
	const stdout: Buffer[] = [];
	const stderr: Buffer[] = [];
	let outputBytes = 0;
	const collect = (chunks: Buffer[], chunk: Buffer) => {
		outputBytes += chunk.byteLength;
		if (outputBytes > MAX_HELPER_OUTPUT_BYTES) {
			child.kill("SIGTERM");
			return;
		}
		chunks.push(chunk);
	};
	child.stdout.on("data", (chunk: Buffer) => collect(stdout, chunk));
	child.stderr.on("data", (chunk: Buffer) => collect(stderr, chunk));

	const status = await new Promise<SubprocessStatus>((resolveExit, reject) => {
		child.once("error", reject);
		child.once("close", (code, signal) => resolveExit({ code, signal }));
	});
	if (outputBytes > MAX_HELPER_OUTPUT_BYTES) {
		throw new WorkflowReconcileError(
			"FlowScript reconcile helper exceeded its 128 MiB output limit.",
		);
	}

	const stdoutText = Buffer.concat(stdout).toString("utf8").trim();
	const stderrText = Buffer.concat(stderr).toString("utf8").trim();
	const parsed = parseJson(stdoutText) as HelperErrorEnvelope | undefined;
	if (status.code !== 0) {
		throw new WorkflowReconcileError(
			subprocessFailureMessage(
				"FlowScript reconcile helper",
				status,
				parsed?.error,
				stderrText,
			),
			Array.isArray(parsed?.diagnostics) ? parsed.diagnostics : [],
		);
	}
	if (
		!parsed ||
		(parsed as { schema?: string }).schema !== WORKFLOW_RENDER_DATA_SCHEMA
	) {
		throw new WorkflowReconcileError(
			`FlowScript reconcile helper returned an invalid result${stderrText ? `: ${stderrText}` : "."}`,
		);
	}
	return parsed as WorkflowRenderData;
}
