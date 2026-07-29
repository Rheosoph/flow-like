import { readFile } from "node:fs/promises";
import { extname, isAbsolute, relative, resolve } from "node:path";
import {
	DOC_SCREENSHOT_PLAN_SCHEMA,
	DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
	type DocScreenshotCaptureStep,
	type DocScreenshotDefaults,
	type DocScreenshotFormat,
	type DocScreenshotPlan,
	type DocScreenshotScenario,
	type DocScreenshotStep,
	type DocScreenshotTauriFixture,
	type DocScreenshotViewport,
	type JsonValue,
} from "./types";

const MAX_SCENARIOS = 20;
const MAX_STEPS = 100;
const MAX_DELAY_MS = 30_000;
const MAX_TIMEOUT_MS = 10 * 60_000;
const MAX_OUTPUT_PIXELS = 100_000_000;
const NAME_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$/;

export const DEFAULT_DOC_SCREENSHOT_OPTIONS: DocScreenshotDefaults = {
	viewport: {
		width: 1624,
		height: 1060,
		deviceScaleFactor: 2,
	},
	theme: "light",
	format: "webp",
	timeoutMs: 120_000,
	settleMs: 350,
	disableAnimations: true,
	hideScrollbars: true,
};

function record(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error(`${label} must be an object.`);
	}
	return value as Record<string, unknown>;
}

function optionalString(value: unknown, label: string): string | undefined {
	if (value === undefined) return undefined;
	if (typeof value !== "string" || value.length === 0) {
		throw new Error(`${label} must be a non-empty string.`);
	}
	return value;
}

function finiteNumber(value: unknown, label: string): number {
	if (typeof value !== "number" || !Number.isFinite(value)) {
		throw new Error(`${label} must be a finite number.`);
	}
	return value;
}

function integerInRange(
	value: unknown,
	label: string,
	min: number,
	max: number,
): number {
	const parsed = finiteNumber(value, label);
	if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
		throw new Error(`${label} must be an integer from ${min} to ${max}.`);
	}
	return parsed;
}

function booleanValue(value: unknown, label: string): boolean {
	if (typeof value !== "boolean") throw new Error(`${label} must be boolean.`);
	return value;
}

function enumValue<T extends string>(
	value: unknown,
	label: string,
	values: readonly T[],
): T {
	if (typeof value !== "string" || !values.includes(value as T)) {
		throw new Error(`${label} must be one of: ${values.join(", ")}.`);
	}
	return value as T;
}

function nameValue(value: unknown, label: string): string {
	const name = optionalString(value, label);
	if (!name || !NAME_PATTERN.test(name)) {
		throw new Error(
			`${label} must start with an alphanumeric character and contain only letters, numbers, dot, underscore, or dash.`,
		);
	}
	return name;
}

function pathValue(value: unknown, label: string): string {
	const path = optionalString(value, label);
	if (!path || !path.startsWith("/") || path.startsWith("//")) {
		throw new Error(`${label} must be a same-origin path beginning with "/".`);
	}
	return path;
}

function formatFromExtension(path: string): DocScreenshotFormat | undefined {
	const extension = extname(path).toLowerCase();
	if (extension === ".png") return "png";
	if (extension === ".webp") return "webp";
	if (extension === ".jpg" || extension === ".jpeg") return "jpeg";
	return undefined;
}

function outputValue(value: unknown, label: string): string | undefined {
	const output = optionalString(value, label);
	if (!output) return undefined;
	if (isAbsolute(output)) {
		throw new Error(`${label} must be relative to outputDir.`);
	}
	const normalized = relative(".", resolve(".", output));
	if (normalized.startsWith("..") || isAbsolute(normalized)) {
		throw new Error(`${label} must stay inside outputDir.`);
	}
	if (!formatFromExtension(output)) {
		throw new Error(`${label} must end in .png, .webp, .jpg, or .jpeg.`);
	}
	return output;
}

function queryValue(
	value: unknown,
	label: string,
): DocScreenshotScenario["query"] {
	if (value === undefined) return undefined;
	const input = record(value, label);
	const output: NonNullable<DocScreenshotScenario["query"]> = {};
	for (const [key, item] of Object.entries(input)) {
		if (!key) throw new Error(`${label} cannot contain an empty key.`);
		const values = Array.isArray(item) ? item : [item];
		for (const entry of values) {
			if (
				entry !== null &&
				typeof entry !== "string" &&
				typeof entry !== "number" &&
				typeof entry !== "boolean"
			) {
				throw new Error(`${label}.${key} must contain only scalar values.`);
			}
		}
		output[key] = Array.isArray(item)
			? (values as NonNullable<(typeof output)[string]>)
			: (item as NonNullable<(typeof output)[string]>);
	}
	return output;
}

function stringRecord(
	value: unknown,
	label: string,
): Record<string, string> | undefined {
	if (value === undefined) return undefined;
	const input = record(value, label);
	const output: Record<string, string> = {};
	for (const [key, item] of Object.entries(input)) {
		if (!key || typeof item !== "string") {
			throw new Error(`${label} must contain string keys and values.`);
		}
		output[key] = item;
	}
	return output;
}

function viewportValue(
	value: unknown,
	label: string,
	fallback: DocScreenshotViewport,
	partial = false,
): DocScreenshotViewport | Partial<DocScreenshotViewport> {
	if (value === undefined) return partial ? {} : fallback;
	const input = record(value, label);
	const width =
		input.width === undefined
			? partial
				? undefined
				: fallback.width
			: integerInRange(input.width, `${label}.width`, 320, 7680);
	const height =
		input.height === undefined
			? partial
				? undefined
				: fallback.height
			: integerInRange(input.height, `${label}.height`, 240, 7680);
	const deviceScaleFactor =
		input.deviceScaleFactor === undefined
			? partial
				? undefined
				: fallback.deviceScaleFactor
			: finiteNumber(input.deviceScaleFactor, `${label}.deviceScaleFactor`);
	if (
		deviceScaleFactor !== undefined &&
		(deviceScaleFactor < 0.5 || deviceScaleFactor > 4)
	) {
		throw new Error(`${label}.deviceScaleFactor must be from 0.5 to 4.`);
	}
	const resolvedWidth = width ?? fallback.width;
	const resolvedHeight = height ?? fallback.height;
	const resolvedDpr = deviceScaleFactor ?? fallback.deviceScaleFactor;
	if (
		resolvedWidth * resolvedHeight * resolvedDpr * resolvedDpr >
		MAX_OUTPUT_PIXELS
	) {
		throw new Error(`${label} exceeds the 100 megapixel capture limit.`);
	}
	return {
		...(width === undefined ? {} : { width }),
		...(height === undefined ? {} : { height }),
		...(deviceScaleFactor === undefined ? {} : { deviceScaleFactor }),
	};
}

function valueInput(
	input: Record<string, unknown>,
	label: string,
): { value?: string; valueEnv?: string } {
	const value = optionalString(input.value, `${label}.value`);
	const valueEnv = optionalString(input.valueEnv, `${label}.valueEnv`);
	if ((value ? 1 : 0) + (valueEnv ? 1 : 0) !== 1) {
		throw new Error(`${label} requires exactly one of value or valueEnv.`);
	}
	if (valueEnv && !/^[A-Z_][A-Z0-9_]*$/.test(valueEnv)) {
		throw new Error(`${label}.valueEnv must be an uppercase environment name.`);
	}
	return { value, valueEnv };
}

function targetValue(
	input: Record<string, unknown>,
	label: string,
): { selector: string; index?: number } {
	const selector = optionalString(input.selector, `${label}.selector`);
	if (!selector) throw new Error(`${label}.selector is required.`);
	const index =
		input.index === undefined
			? undefined
			: integerInRange(input.index, `${label}.index`, 0, 999);
	return { selector, index };
}

function validateCaptureStep(
	input: Record<string, unknown>,
	label: string,
): DocScreenshotCaptureStep {
	const name = nameValue(input.name, `${label}.name`);
	const mode =
		input.mode === undefined
			? ("viewport" as const)
			: enumValue(input.mode, `${label}.mode`, [
					"viewport",
					"fullPage",
					"element",
				] as const);
	const selector = optionalString(input.selector, `${label}.selector`);
	if (mode === "element" && !selector) {
		throw new Error(`${label}.selector is required for element capture.`);
	}
	if (mode !== "element" && selector) {
		throw new Error(`${label}.selector is only valid for element capture.`);
	}
	const index =
		input.index === undefined
			? undefined
			: integerInRange(input.index, `${label}.index`, 0, 999);
	const padding =
		input.padding === undefined
			? undefined
			: integerInRange(input.padding, `${label}.padding`, 0, 512);
	const output = outputValue(input.output, `${label}.output`);
	const format =
		input.format === undefined
			? undefined
			: enumValue(input.format, `${label}.format`, [
					"png",
					"webp",
					"jpeg",
				] as const);
	const extensionFormat = output ? formatFromExtension(output) : undefined;
	if (format && extensionFormat && format !== extensionFormat) {
		throw new Error(`${label}.format does not match its output extension.`);
	}
	const quality =
		input.quality === undefined
			? undefined
			: integerInRange(input.quality, `${label}.quality`, 1, 100);
	if (quality !== undefined && (format ?? extensionFormat) !== "jpeg") {
		throw new Error(`${label}.quality is supported only for JPEG.`);
	}
	let hideSelectors: string[] | undefined;
	if (input.hideSelectors !== undefined) {
		if (
			!Array.isArray(input.hideSelectors) ||
			!input.hideSelectors.every(
				(item) => typeof item === "string" && item.length > 0,
			)
		) {
			throw new Error(`${label}.hideSelectors must be an array of selectors.`);
		}
		hideSelectors = input.hideSelectors;
	}
	return {
		type: "capture",
		name,
		output,
		mode,
		selector,
		index,
		padding,
		format,
		quality,
		hideSelectors,
	};
}

function validateStep(value: unknown, label: string): DocScreenshotStep {
	const input = record(value, label);
	const type = optionalString(input.type, `${label}.type`);
	switch (type) {
		case "goto":
			return {
				type,
				path: pathValue(input.path, `${label}.path`),
				query: queryValue(input.query, `${label}.query`),
			};
		case "click": {
			const target = targetValue(input, label);
			return {
				type,
				...target,
				button:
					input.button === undefined
						? undefined
						: enumValue(input.button, `${label}.button`, [
								"left",
								"middle",
								"right",
							] as const),
				clickCount:
					input.clickCount === undefined
						? undefined
						: integerInRange(input.clickCount, `${label}.clickCount`, 1, 3),
			};
		}
		case "fill":
			return {
				type,
				...targetValue(input, label),
				...valueInput(input, label),
			};
		case "type":
			return {
				type,
				...targetValue(input, label),
				...valueInput(input, label),
				delayMs:
					input.delayMs === undefined
						? undefined
						: integerInRange(input.delayMs, `${label}.delayMs`, 0, 1000),
			};
		case "press":
			return {
				type,
				key: optionalString(input.key, `${label}.key`) ?? "",
				selector: optionalString(input.selector, `${label}.selector`),
				index:
					input.index === undefined
						? undefined
						: integerInRange(input.index, `${label}.index`, 0, 999),
			};
		case "select": {
			const target = targetValue(input, label);
			if (
				!Array.isArray(input.values) ||
				input.values.length === 0 ||
				!input.values.every((item) => typeof item === "string")
			) {
				throw new Error(`${label}.values must be a non-empty string array.`);
			}
			return { type, ...target, values: input.values };
		}
		case "check":
			return {
				type,
				...targetValue(input, label),
				checked:
					input.checked === undefined
						? undefined
						: booleanValue(input.checked, `${label}.checked`),
			};
		case "hover":
			return { type, ...targetValue(input, label) };
		case "scroll":
			return {
				type,
				selector: optionalString(input.selector, `${label}.selector`),
				index:
					input.index === undefined
						? undefined
						: integerInRange(input.index, `${label}.index`, 0, 999),
				x:
					input.x === undefined
						? undefined
						: finiteNumber(input.x, `${label}.x`),
				y:
					input.y === undefined
						? undefined
						: finiteNumber(input.y, `${label}.y`),
			};
		case "waitFor": {
			const selector = optionalString(input.selector, `${label}.selector`);
			const urlIncludes = optionalString(
				input.urlIncludes,
				`${label}.urlIncludes`,
			);
			const text = optionalString(input.text, `${label}.text`);
			if ([selector, urlIncludes, text].filter(Boolean).length !== 1) {
				throw new Error(
					`${label} requires exactly one of selector, urlIncludes, or text.`,
				);
			}
			return {
				type,
				selector,
				urlIncludes,
				text,
				state:
					input.state === undefined
						? undefined
						: enumValue(input.state, `${label}.state`, [
								"attached",
								"visible",
								"hidden",
								"detached",
							] as const),
				timeoutMs:
					input.timeoutMs === undefined
						? undefined
						: integerInRange(
								input.timeoutMs,
								`${label}.timeoutMs`,
								1,
								MAX_TIMEOUT_MS,
							),
			};
		}
		case "delay":
			return {
				type,
				ms: integerInRange(input.ms, `${label}.ms`, 0, MAX_DELAY_MS),
			};
		case "capture":
			return validateCaptureStep(input, label);
		default:
			throw new Error(`${label}.type is not supported: ${String(type)}`);
	}
}

function defaultsValue(value: unknown): DocScreenshotDefaults {
	if (value === undefined)
		return structuredClone(DEFAULT_DOC_SCREENSHOT_OPTIONS);
	const input = record(value, "plan.defaults");
	const format =
		input.format === undefined
			? DEFAULT_DOC_SCREENSHOT_OPTIONS.format
			: enumValue(input.format, "plan.defaults.format", [
					"png",
					"webp",
					"jpeg",
				] as const);
	const quality =
		input.quality === undefined
			? undefined
			: integerInRange(input.quality, "plan.defaults.quality", 1, 100);
	if (quality !== undefined && format !== "jpeg") {
		throw new Error("plan.defaults.quality is supported only for JPEG.");
	}
	return {
		viewport: viewportValue(
			input.viewport,
			"plan.defaults.viewport",
			DEFAULT_DOC_SCREENSHOT_OPTIONS.viewport,
		) as DocScreenshotViewport,
		theme:
			input.theme === undefined
				? DEFAULT_DOC_SCREENSHOT_OPTIONS.theme
				: enumValue(input.theme, "plan.defaults.theme", [
						"light",
						"dark",
					] as const),
		format,
		quality,
		timeoutMs:
			input.timeoutMs === undefined
				? DEFAULT_DOC_SCREENSHOT_OPTIONS.timeoutMs
				: integerInRange(
						input.timeoutMs,
						"plan.defaults.timeoutMs",
						1,
						MAX_TIMEOUT_MS,
					),
		settleMs:
			input.settleMs === undefined
				? DEFAULT_DOC_SCREENSHOT_OPTIONS.settleMs
				: integerInRange(input.settleMs, "plan.defaults.settleMs", 0, 30_000),
		disableAnimations:
			input.disableAnimations === undefined
				? DEFAULT_DOC_SCREENSHOT_OPTIONS.disableAnimations
				: booleanValue(
						input.disableAnimations,
						"plan.defaults.disableAnimations",
					),
		hideScrollbars:
			input.hideScrollbars === undefined
				? DEFAULT_DOC_SCREENSHOT_OPTIONS.hideScrollbars
				: booleanValue(input.hideScrollbars, "plan.defaults.hideScrollbars"),
	};
}

export function validateDocScreenshotPlan(value: unknown): DocScreenshotPlan {
	const input = record(value, "plan");
	if (input.schema !== DOC_SCREENSHOT_PLAN_SCHEMA) {
		throw new Error(`plan.schema must be ${DOC_SCREENSHOT_PLAN_SCHEMA}.`);
	}
	const defaults = defaultsValue(input.defaults);
	const app = enumValue(input.app ?? "desktop", "plan.app", [
		"desktop",
		"web",
	] as const);
	const outputDir =
		optionalString(input.outputDir, "plan.outputDir") ?? "tmp/doc-screenshots";
	const baseUrl = optionalString(input.baseUrl, "plan.baseUrl");
	const tauriFixture = optionalString(input.tauriFixture, "plan.tauriFixture");
	if (!Array.isArray(input.scenarios) || input.scenarios.length === 0) {
		throw new Error("plan.scenarios must be a non-empty array.");
	}
	if (input.scenarios.length > MAX_SCENARIOS) {
		throw new Error(`plan.scenarios cannot exceed ${MAX_SCENARIOS}.`);
	}
	const scenarioNames = new Set<string>();
	const captureNames = new Set<string>();
	const scenarios = input.scenarios.map((scenarioValue, scenarioIndex) => {
		const label = `plan.scenarios[${scenarioIndex}]`;
		const scenarioInput = record(scenarioValue, label);
		const name = nameValue(scenarioInput.name, `${label}.name`);
		if (scenarioNames.has(name))
			throw new Error(`Duplicate scenario name: ${name}`);
		scenarioNames.add(name);
		if (
			!Array.isArray(scenarioInput.steps) ||
			scenarioInput.steps.length === 0
		) {
			throw new Error(`${label}.steps must be a non-empty array.`);
		}
		if (scenarioInput.steps.length > MAX_STEPS) {
			throw new Error(`${label}.steps cannot exceed ${MAX_STEPS}.`);
		}
		const steps = scenarioInput.steps.map((step, stepIndex) =>
			validateStep(step, `${label}.steps[${stepIndex}]`),
		);
		if (!steps.some((step) => step.type === "capture")) {
			throw new Error(`${label} must contain at least one capture step.`);
		}
		for (const step of steps) {
			if (step.type !== "capture") continue;
			if (captureNames.has(step.name)) {
				throw new Error(`Duplicate capture name: ${step.name}`);
			}
			captureNames.add(step.name);
		}
		const viewport = viewportValue(
			scenarioInput.viewport,
			`${label}.viewport`,
			defaults.viewport,
			true,
		) as Partial<DocScreenshotViewport>;
		return {
			name,
			path: pathValue(scenarioInput.path, `${label}.path`),
			query: queryValue(scenarioInput.query, `${label}.query`),
			viewport: Object.keys(viewport).length === 0 ? undefined : viewport,
			theme:
				scenarioInput.theme === undefined
					? undefined
					: enumValue(scenarioInput.theme, `${label}.theme`, [
							"light",
							"dark",
						] as const),
			localStorage: stringRecord(
				scenarioInput.localStorage,
				`${label}.localStorage`,
			),
			sessionStorage: stringRecord(
				scenarioInput.sessionStorage,
				`${label}.sessionStorage`,
			),
			steps,
		} satisfies DocScreenshotScenario;
	});
	return {
		schema: DOC_SCREENSHOT_PLAN_SCHEMA,
		app,
		baseUrl,
		outputDir,
		tauriFixture,
		defaults,
		scenarios,
	};
}

export async function loadDocScreenshotPlan(
	path: string,
): Promise<DocScreenshotPlan> {
	let parsed: unknown;
	try {
		parsed = JSON.parse(await readFile(path, "utf8"));
	} catch (error) {
		throw new Error(`Could not read screenshot plan ${path}: ${String(error)}`);
	}
	return validateDocScreenshotPlan(parsed);
}

function isJsonValue(value: unknown): value is JsonValue {
	if (
		value === null ||
		typeof value === "string" ||
		typeof value === "boolean" ||
		(typeof value === "number" && Number.isFinite(value))
	) {
		return true;
	}
	if (Array.isArray(value)) return value.every(isJsonValue);
	if (!value || typeof value !== "object") return false;
	return Object.values(value).every(isJsonValue);
}

export function validateDocScreenshotTauriFixture(
	value: unknown,
): DocScreenshotTauriFixture {
	const input = record(value, "fixture");
	if (input.schema !== DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA) {
		throw new Error(
			`fixture.schema must be ${DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA}.`,
		);
	}
	const responseInput = record(input.responses, "fixture.responses");
	const responses: DocScreenshotTauriFixture["responses"] = {};
	for (const [command, response] of Object.entries(responseInput)) {
		if (!command)
			throw new Error("fixture.responses cannot have an empty command.");
		if (!isJsonValue(response)) {
			throw new Error(`fixture.responses.${command} is not JSON-serializable.`);
		}
		responses[command] = response;
	}
	return {
		schema: DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
		strict:
			input.strict === undefined
				? true
				: booleanValue(input.strict, "fixture.strict"),
		responses,
	};
}

export async function loadDocScreenshotTauriFixture(
	path: string,
): Promise<DocScreenshotTauriFixture> {
	let parsed: unknown;
	try {
		parsed = JSON.parse(await readFile(path, "utf8"));
	} catch (error) {
		throw new Error(`Could not read Tauri fixture ${path}: ${String(error)}`);
	}
	return validateDocScreenshotTauriFixture(parsed);
}

export function outputFormatForCapture(
	step: DocScreenshotCaptureStep,
	defaultFormat: DocScreenshotFormat,
): DocScreenshotFormat {
	return (
		step.format ??
		(step.output ? formatFromExtension(step.output) : undefined) ??
		defaultFormat
	);
}

export function safeCaptureOutputPath(
	outputDir: string,
	step: DocScreenshotCaptureStep,
	format: DocScreenshotFormat,
): string {
	const extension = format === "jpeg" ? "jpg" : format;
	const requested = step.output ?? `${step.name}.${extension}`;
	const absoluteDir = resolve(outputDir);
	const absoluteOutput = resolve(absoluteDir, requested);
	const relativeOutput = relative(absoluteDir, absoluteOutput);
	if (
		relativeOutput === "" ||
		relativeOutput.startsWith("..") ||
		isAbsolute(relativeOutput)
	) {
		throw new Error(`Capture output escapes outputDir: ${requested}`);
	}
	return absoluteOutput;
}
