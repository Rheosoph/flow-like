import { basename, extname, resolve } from "node:path";
import type { LayoutStyle } from "@flow-like/flow-like-ui/lib/flow-auto-layout";
import type {
	WorkflowScreenshotCliOptions,
	WorkflowScreenshotFormat,
} from "./types";

function valueAfter(
	args: string[],
	index: number,
	flag: string,
): [string, number] {
	const current = args[index] ?? "";
	if (current.startsWith(`${flag}=`)) {
		const value = current.slice(flag.length + 1);
		if (!value) throw new Error(`${flag} requires a value.`);
		return [value, index];
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

function boundedNumber(
	value: string,
	flag: string,
	min: number,
	max: number,
): number {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < min || parsed > max) {
		throw new Error(`${flag} must be from ${min} to ${max}.`);
	}
	return parsed;
}

function parseViewport(value: string): { width: number; height: number } {
	const match = value.match(/^(\d+)x(\d+)$/i);
	if (!match) throw new Error("--viewport must use WIDTHxHEIGHT.");
	const width = positiveInteger(match[1] ?? "", "--viewport width");
	const height = positiveInteger(match[2] ?? "", "--viewport height");
	if (width < 320 || width > 7680 || height < 240 || height > 7680) {
		throw new Error(
			"--viewport is outside the supported 320x240–7680x7680 range.",
		);
	}
	return { width, height };
}

export function inferWorkflowScreenshotFormat(
	path: string,
): WorkflowScreenshotFormat | undefined {
	const extension = extname(path).toLowerCase();
	if (extension === ".webp") return "webp";
	if (extension === ".png") return "png";
	if (extension === ".jpg" || extension === ".jpeg") return "jpeg";
	return undefined;
}

function defaultOutput(input: string): string {
	const extension = extname(input);
	const stem = basename(input, extension)
		.replace(/[^a-zA-Z0-9._-]+/g, "-")
		.replace(/^-+|-+$/g, "")
		.slice(0, 100);
	return resolve(
		process.cwd(),
		"tmp/workflow-screenshots",
		`${stem || "workflow"}.webp`,
	);
}

export function parseWorkflowScreenshotArgs(
	args: string[],
): WorkflowScreenshotCliOptions {
	const options: WorkflowScreenshotCliOptions = {
		layout: "balanced",
		listNodes: false,
		viewport: { width: 1624, height: 1060 },
		dpr: 2,
		theme: "dark",
		timeoutMs: 120_000,
		settleMs: 650,
		json: false,
		help: false,
	};
	const normalizedArgs = args.filter((arg) => arg !== "--");
	for (let index = 0; index < normalizedArgs.length; index += 1) {
		const arg = normalizedArgs[index] ?? "";
		if (arg === "--help" || arg === "-h") options.help = true;
		else if (arg === "--json") options.json = true;
		else if (arg === "--list-nodes") options.listNodes = true;
		else if (arg === "--output" || arg.startsWith("--output=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--output");
			index = consumed;
			options.output = resolve(process.cwd(), value);
		} else if (arg === "--name" || arg.startsWith("--name=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--name");
			index = consumed;
			options.name = value;
		} else if (arg === "--layout" || arg.startsWith("--layout=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--layout");
			index = consumed;
			if (!(["compact", "balanced", "expanded"] as string[]).includes(value)) {
				throw new Error("--layout must be compact, balanced, or expanded.");
			}
			options.layout = value as LayoutStyle;
		} else if (arg === "--focus-node" || arg.startsWith("--focus-node=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--focus-node",
			);
			index = consumed;
			options.focusNode = value;
		} else if (arg === "--viewport" || arg.startsWith("--viewport=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--viewport");
			index = consumed;
			options.viewport = parseViewport(value);
		} else if (arg === "--dpr" || arg.startsWith("--dpr=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--dpr");
			index = consumed;
			options.dpr = boundedNumber(value, "--dpr", 0.5, 4);
		} else if (arg === "--theme" || arg.startsWith("--theme=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--theme");
			index = consumed;
			if (value !== "light" && value !== "dark") {
				throw new Error("--theme must be light or dark.");
			}
			options.theme = value;
		} else if (arg === "--quality" || arg.startsWith("--quality=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--quality");
			index = consumed;
			options.quality = positiveInteger(value, "--quality");
			if (options.quality > 100) {
				throw new Error("--quality cannot exceed 100.");
			}
		} else if (arg === "--frontend-url" || arg.startsWith("--frontend-url=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--frontend-url",
			);
			index = consumed;
			options.frontendUrl = value;
		} else if (arg === "--port" || arg.startsWith("--port=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--port");
			index = consumed;
			options.port = positiveInteger(value, "--port");
			if (options.port > 65_535) throw new Error("--port cannot exceed 65535.");
		} else if (arg === "--timeout-ms" || arg.startsWith("--timeout-ms=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--timeout-ms",
			);
			index = consumed;
			options.timeoutMs = positiveInteger(value, "--timeout-ms");
		} else if (arg === "--settle-ms" || arg.startsWith("--settle-ms=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--settle-ms",
			);
			index = consumed;
			options.settleMs = positiveInteger(value, "--settle-ms");
		} else if (arg.startsWith("-")) {
			throw new Error(`Unknown argument: ${arg}`);
		} else if (options.input) {
			throw new Error(`Unexpected second FlowScript input: ${arg}`);
		} else {
			options.input = resolve(process.cwd(), arg);
		}
	}

	if (!options.help) {
		if (!options.input) throw new Error("A FlowScript input file is required.");
		options.output ??= defaultOutput(options.input);
		const format = inferWorkflowScreenshotFormat(options.output);
		if (!format) {
			throw new Error("--output must end in .webp, .png, .jpg, or .jpeg.");
		}
		if (options.quality !== undefined && format !== "jpeg") {
			throw new Error("--quality requires JPEG output.");
		}
	}

	return options;
}
