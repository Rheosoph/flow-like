import { resolve } from "node:path";

export type UniversityCliMode = "apply" | "inspect" | "list" | "asset";

export type UniversityCliAssetKind = "IMAGE" | "VIDEO" | "AUDIO" | "DOCUMENT";

export interface UniversityCliOptions {
	mode?: UniversityCliMode;
	plan?: string;
	inspectCourseId?: string;
	assetCourseId?: string;
	assetName?: string;
	assetFile?: string;
	assetKind?: UniversityCliAssetKind;
	assetMimeType?: string;
	language?: string;
	apiUrl?: string;
	timeoutMs?: number;
	dryRun: boolean;
	replaceAsset: boolean;
	json: boolean;
	help: boolean;
}

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

function setMode(
	options: UniversityCliOptions,
	mode: UniversityCliMode,
	flag: string,
): void {
	if (options.mode && options.mode !== mode) {
		throw new Error(
			`${flag} cannot be combined with the selected ${options.mode} mode.`,
		);
	}
	options.mode = mode;
}

export function parseUniversityArgs(args: string[]): UniversityCliOptions {
	const options: UniversityCliOptions = {
		dryRun: false,
		replaceAsset: false,
		json: false,
		help: false,
	};
	const normalizedArgs = args.filter((arg) => arg !== "--");

	for (let index = 0; index < normalizedArgs.length; index += 1) {
		const arg = normalizedArgs[index] ?? "";
		if (arg === "--help" || arg === "-h") options.help = true;
		else if (arg === "--json") options.json = true;
		else if (arg === "--dry-run") options.dryRun = true;
		else if (arg === "--replace") options.replaceAsset = true;
		else if (arg === "--list") setMode(options, "list", "--list");
		else if (arg === "--plan" || arg.startsWith("--plan=")) {
			setMode(options, "apply", "--plan");
			const [value, consumed] = valueAfter(normalizedArgs, index, "--plan");
			index = consumed;
			options.plan = resolve(process.cwd(), value);
		} else if (arg === "--inspect" || arg.startsWith("--inspect=")) {
			setMode(options, "inspect", "--inspect");
			const [value, consumed] = valueAfter(normalizedArgs, index, "--inspect");
			index = consumed;
			options.inspectCourseId = value;
		} else if (arg === "--asset" || arg.startsWith("--asset=")) {
			setMode(options, "asset", "--asset");
			const [value, consumed] = valueAfter(normalizedArgs, index, "--asset");
			index = consumed;
			options.assetCourseId = value;
		} else if (arg === "--name" || arg.startsWith("--name=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--name");
			index = consumed;
			options.assetName = value;
		} else if (arg === "--file" || arg.startsWith("--file=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--file");
			index = consumed;
			options.assetFile = resolve(process.cwd(), value);
		} else if (arg === "--kind" || arg.startsWith("--kind=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--kind");
			index = consumed;
			const kind = value.toUpperCase();
			if (!["IMAGE", "VIDEO", "AUDIO", "DOCUMENT"].includes(kind)) {
				throw new Error("--kind must be IMAGE, VIDEO, AUDIO, or DOCUMENT.");
			}
			options.assetKind = kind as UniversityCliAssetKind;
		} else if (arg === "--mime-type" || arg.startsWith("--mime-type=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--mime-type",
			);
			index = consumed;
			options.assetMimeType = value;
		} else if (arg === "--language" || arg.startsWith("--language=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--language");
			index = consumed;
			options.language = value;
		} else if (arg === "--api-url" || arg.startsWith("--api-url=")) {
			const [value, consumed] = valueAfter(normalizedArgs, index, "--api-url");
			index = consumed;
			options.apiUrl = value;
		} else if (arg === "--timeout-ms" || arg.startsWith("--timeout-ms=")) {
			const [value, consumed] = valueAfter(
				normalizedArgs,
				index,
				"--timeout-ms",
			);
			index = consumed;
			options.timeoutMs = positiveInteger(value, "--timeout-ms");
			if (options.timeoutMs > 300_000) {
				throw new Error("--timeout-ms cannot exceed 300000.");
			}
		} else {
			throw new Error(`Unknown argument: ${arg}`);
		}
	}

	if (options.help) return options;
	if (!options.mode) {
		throw new Error("Select one of --plan, --inspect, --list, or --asset.");
	}
	if (options.mode === "apply" && !options.plan) {
		throw new Error("Apply mode requires --plan.");
	}
	if (options.mode !== "apply" && options.dryRun) {
		throw new Error("--dry-run can only be used with --plan.");
	}
	if (options.mode === "asset") {
		if (!options.assetCourseId || !options.assetName || !options.assetFile) {
			throw new Error(
				"Asset mode requires --asset <course-id>, --name, and --file.",
			);
		}
		if (!/^[A-Za-z_][A-Za-z0-9_-]{0,63}$/.test(options.assetName)) {
			throw new Error(
				"--name must start with a letter or underscore and contain at most 64 letters, digits, underscores, or dashes.",
			);
		}
		if (
			options.assetMimeType &&
			!/^[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]*\/[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]*$/.test(
				options.assetMimeType,
			)
		) {
			throw new Error("--mime-type must be a valid MIME type.");
		}
	} else if (
		options.assetName ||
		options.assetFile ||
		options.assetKind ||
		options.assetMimeType ||
		options.replaceAsset
	) {
		throw new Error(
			"--name, --file, --kind, --mime-type, and --replace require --asset.",
		);
	}
	if (
		options.language &&
		options.mode !== "inspect" &&
		options.mode !== "list"
	) {
		throw new Error("--language can only be used with --inspect or --list.");
	}

	return options;
}
