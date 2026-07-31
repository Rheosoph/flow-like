import { basename, dirname, extname, resolve } from "node:path";
import {
	DEFAULT_DOC_SCREENSHOT_OPTIONS,
	validateDocScreenshotPlan,
} from "./plan";
import {
	DOC_SCREENSHOT_PLAN_SCHEMA,
	type DocScreenshotApp,
	type DocScreenshotFormat,
	type DocScreenshotPlan,
	type DocScreenshotTheme,
} from "./types";

export interface DocScreenshotCliOptions {
	app: DocScreenshotApp;
	plan?: string;
	path?: string;
	query: Array<[string, string]>;
	output?: string;
	outputDir?: string;
	frontendUrl?: string;
	port?: number;
	viewport?: { width: number; height: number };
	dpr?: number;
	theme?: DocScreenshotTheme;
	format?: DocScreenshotFormat;
	quality?: number;
	fullPage: boolean;
	selector?: string;
	waitFor?: string;
	timeoutMs?: number;
	settleMs?: number;
	json: boolean;
	keepServer: boolean;
	help: boolean;
}

const valueAfter = (
	args: string[],
	index: number,
	flag: string,
): [string, number] => {
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
};

const positiveInteger = (value: string, flag: string): number => {
	const parsed = Number(value);
	if (!Number.isSafeInteger(parsed) || parsed < 1) {
		throw new Error(`${flag} must be a positive integer.`);
	}
	return parsed;
};

const boundedNumber = (
	value: string,
	flag: string,
	min: number,
	max: number,
): number => {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < min || parsed > max) {
		throw new Error(`${flag} must be from ${min} to ${max}.`);
	}
	return parsed;
};

const parseViewport = (value: string): { width: number; height: number } => {
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
};

export function parseDocScreenshotArgs(
	args: string[],
): DocScreenshotCliOptions {
	const options: DocScreenshotCliOptions = {
		app: "desktop",
		query: [],
		fullPage: false,
		json: false,
		keepServer: false,
		help: false,
	};
	const normalizedArgs = args.filter((arg) => arg !== "--");
	for (let index = 0; index < normalizedArgs.length; index += 1) {
		const arg = normalizedArgs[index] ?? "";
		if (arg === "--help" || arg === "-h") options.help = true;
		else if (arg === "--json") options.json = true;
		else if (arg === "--keep-server") options.keepServer = true;
		else if (arg === "--full-page") options.fullPage = true;
		else if (arg === "--app" || arg.startsWith("--app=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--app");
			index = consumed;
			if (value !== "desktop" && value !== "web") {
				throw new Error("--app must be desktop or web.");
			}
			options.app = value;
		} else if (arg === "--plan" || arg.startsWith("--plan=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--plan");
			index = consumed;
			options.plan = resolve(process.cwd(), value);
		} else if (arg === "--path" || arg.startsWith("--path=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--path");
			index = consumed;
			options.path = value;
		} else if (arg === "--query" || arg.startsWith("--query=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--query");
			index = consumed;
			const separator = value.indexOf("=");
			if (separator < 1) throw new Error("--query must use KEY=VALUE.");
			options.query.push([
				value.slice(0, separator),
				value.slice(separator + 1),
			]);
		} else if (arg === "--output" || arg.startsWith("--output=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--output");
			index = consumed;
			options.output = resolve(process.cwd(), value);
		} else if (arg === "--output-dir" || arg.startsWith("--output-dir=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--output-dir",
			);
			index = consumed;
			options.outputDir = resolve(process.cwd(), value);
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
		} else if (arg === "--format" || arg.startsWith("--format=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--format");
			index = consumed;
			if (value !== "png" && value !== "webp" && value !== "jpeg") {
				throw new Error("--format must be png, webp, or jpeg.");
			}
			options.format = value;
		} else if (arg === "--quality" || arg.startsWith("--quality=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--quality");
			index = consumed;
			options.quality = positiveInteger(value, "--quality");
			if (options.quality > 100) {
				throw new Error("--quality cannot exceed 100.");
			}
		} else if (arg === "--selector" || arg.startsWith("--selector=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--selector");
			index = consumed;
			options.selector = value;
		} else if (arg === "--wait-for" || arg.startsWith("--wait-for=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--wait-for");
			index = consumed;
			options.waitFor = value;
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
		} else {
			throw new Error(`Unknown argument: ${arg}`);
		}
	}

	if (options.plan) {
		const directFlags = [
			options.path,
			options.output,
			options.viewport,
			options.dpr,
			options.theme,
			options.format,
			options.quality,
			options.selector,
			options.waitFor,
			options.timeoutMs,
			options.settleMs,
			options.fullPage || undefined,
			options.query.length > 0 || undefined,
		].filter((value) => value !== undefined);
		if (directFlags.length > 0) {
			throw new Error(
				"--plan cannot be combined with direct capture options; only --frontend-url, --output-dir, --port, --json, and --keep-server may override a plan.",
			);
		}
	} else if (!options.help) {
		if (!options.path) throw new Error("Direct mode requires --path.");
		if (options.fullPage && options.selector) {
			throw new Error("Use either --full-page or --selector, not both.");
		}
	}
	return options;
}

function inferFormat(path: string): DocScreenshotFormat | undefined {
	const extension = extname(path).toLowerCase();
	if (extension === ".png") return "png";
	if (extension === ".webp") return "webp";
	if (extension === ".jpg" || extension === ".jpeg") return "jpeg";
	return undefined;
}

function directName(path: string): string {
	const withoutQuery = path.split(/[?#]/, 1)[0] ?? "";
	const raw = basename(withoutQuery) || "home";
	return (
		raw
			.replace(/[^a-zA-Z0-9._-]+/g, "-")
			.replace(/^-+|-+$/g, "")
			.slice(0, 80) || "capture"
	);
}

export function directPlanFromOptions(
	options: DocScreenshotCliOptions,
): DocScreenshotPlan {
	if (!options.path) throw new Error("Direct mode requires --path.");
	const name = directName(options.path);
	const requestedOutput =
		options.output ??
		resolve(
			process.cwd(),
			"tmp/doc-screenshots",
			`${name}.${options.format === "jpeg" ? "jpg" : (options.format ?? "webp")}`,
		);
	const inferredFormat = inferFormat(requestedOutput);
	if (!inferredFormat) {
		throw new Error("--output must end in .png, .webp, .jpg, or .jpeg.");
	}
	if (options.format && options.format !== inferredFormat) {
		throw new Error("--format does not match the --output extension.");
	}
	if (options.quality !== undefined && inferredFormat !== "jpeg") {
		throw new Error("--quality requires JPEG output.");
	}
	const query: Record<string, string[]> = {};
	for (const [key, value] of options.query) {
		const current = query[key];
		if (current) current.push(value);
		else query[key] = [value];
	}
	const captureMode = options.selector
		? ("element" as const)
		: options.fullPage
			? ("fullPage" as const)
			: ("viewport" as const);
	const steps: DocScreenshotPlan["scenarios"][number]["steps"] = [];
	if (options.waitFor) {
		steps.push({
			type: "waitFor",
			selector: options.waitFor,
			state: "visible",
		});
	}
	steps.push({
		type: "capture",
		name,
		output: basename(requestedOutput),
		mode: captureMode,
		selector: options.selector,
		format: inferredFormat,
		quality: options.quality,
	});
	const defaults = structuredClone(DEFAULT_DOC_SCREENSHOT_OPTIONS);
	defaults.viewport = {
		width: options.viewport?.width ?? defaults.viewport.width,
		height: options.viewport?.height ?? defaults.viewport.height,
		deviceScaleFactor: options.dpr ?? defaults.viewport.deviceScaleFactor,
	};
	defaults.theme = options.theme ?? defaults.theme;
	defaults.format = inferredFormat;
	defaults.quality = options.quality;
	defaults.timeoutMs = options.timeoutMs ?? defaults.timeoutMs;
	defaults.settleMs = options.settleMs ?? defaults.settleMs;
	return validateDocScreenshotPlan({
		schema: DOC_SCREENSHOT_PLAN_SCHEMA,
		app: options.app,
		outputDir: options.outputDir ?? dirname(requestedOutput),
		defaults,
		scenarios: [
			{
				name,
				path: options.path,
				query,
				steps,
			},
		],
	});
}
