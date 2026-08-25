#!/usr/bin/env bun

import { platform } from "node:os";
import { parseWorkflowScreenshotArgs } from "../lib/workflow-screenshot/cli";
import { WorkflowReconcileError } from "../lib/workflow-screenshot/reconcile";
import {
	type WorkflowNodeListResult,
	runWorkflowScreenshot,
} from "../lib/workflow-screenshot/runner";
import {
	WORKFLOW_NODE_LIST_SCHEMA,
	type WorkflowScreenshotCliOptions,
	type WorkflowScreenshotResult,
} from "../lib/workflow-screenshot/types";

export function usage(): string {
	return `Flow-Like workflow screenshot CLI

Reconcile FlowScript with the real node catalog, apply Studio's auto-layout, open the
result in the desktop frontend, and capture a deterministic workflow screenshot.

Usage:
  bun run workflow:screenshot -- path/to/workflow.flow --output path/to/workflow.webp
  bun run workflow:screenshot -- workflow.flow --layout balanced --focus-node normalize
  bun run workflow:screenshot -- workflow.flow --list-nodes

Workflow:
  --output <path>         .webp, .png, .jpg, or .jpeg output path
  --name <title>          Board title shown in Studio (default: input filename)
  --layout <style>        compact, balanced (default), or expanded
  --focus-node <selector> Focus an id, //@n:/@l: anchor, or unique node/layer name
  --handle-errors <node>  Add Studio's error outputs; focus it unless overridden
  --list-nodes            Reconcile and print focusable ids without opening Chromium

Rendering:
  --viewport <WxH>        CSS viewport (default: 1624x1060)
  --dpr <number>          Device scale factor, 0.5-4 (default: 2)
  --theme <light|dark>    App color scheme (default: dark)
  --quality <1-100>       JPEG quality; only valid for .jpg/.jpeg output
  --timeout-ms <number>   Navigation/action timeout (default: 120000)
  --settle-ms <number>    Delay after fit/focus before capture (default: 650)

Frontend:
  --frontend-url <url>    Reuse an existing loopback desktop frontend
  --port <number>         Pin the automatically started frontend to a specific port
  --json                  Reserve stdout for a machine-readable result
  --help                  Show this help

The command uses an ephemeral in-memory app/board fixture and never writes the reconciled
workflow into a real Flow-Like profile.`;
}

function printNodeList(result: WorkflowNodeListResult): void {
	console.log(`Focusable workflow items in ${result.board.name}:`);
	if (result.nodes.length === 0) {
		console.log("  (none)");
		return;
	}
	for (const node of result.nodes) {
		const layer = node.layer ? ` layer=${node.layer}` : "";
		const label = node.friendlyName
			? `${node.friendlyName} [${node.name}]`
			: node.name;
		console.log(`  ${node.kind.padEnd(5)} ${node.id}  ${label}${layer}`);
	}
}

function printScreenshot(result: WorkflowScreenshotResult): void {
	const artifact = result.screenshot.scenarios[0]?.artifacts[0];
	const dimensions = artifact
		? `${artifact.pixels.width}x${artifact.pixels.height}`
		: "captured";
	console.log(`PASS workflow screenshot: ${dimensions} ${result.output}`);
	console.log(
		`  ${result.board.nodes} node(s), ${result.board.layers} layer(s), ${result.layout} layout${result.focus ? `, focused ${result.focus.label} (${result.focus.id})` : ""}`,
	);
}

export async function main(args = Bun.argv.slice(2)): Promise<number> {
	let options: WorkflowScreenshotCliOptions;
	try {
		options = parseWorkflowScreenshotArgs(args);
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
		if (!options.json) {
			console.error(
				`Reconciling ${options.input} with the Flow-Like catalog...`,
			);
		}
		const result = await runWorkflowScreenshot(options);
		if (options.json) console.log(JSON.stringify(result));
		else if (result.schema === WORKFLOW_NODE_LIST_SCHEMA) printNodeList(result);
		else printScreenshot(result);
		return 0;
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		const diagnostics =
			error instanceof WorkflowReconcileError ? error.diagnostics : [];
		if (options.json) {
			console.log(
				JSON.stringify({
					schema: "flow-like.workflow-screenshot-error/v1",
					passed: false,
					error: message,
					diagnostics,
					platform: platform(),
				}),
			);
		} else {
			console.error(`Workflow screenshot failed: ${message}`);
			for (const diagnostic of diagnostics) console.error(`  - ${diagnostic}`);
			if (diagnostics.length > 0) {
				console.error(
					"Run with --list-nodes only after these diagnostics are fixed.",
				);
			}
		}
		return 1;
	}
}

if (import.meta.main) process.exitCode = await main();
