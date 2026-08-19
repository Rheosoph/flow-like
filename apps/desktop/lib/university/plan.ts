import { readFile, stat } from "node:fs/promises";
import {
	basename,
	dirname,
	extname,
	isAbsolute,
	normalize,
	resolve,
} from "node:path";
import sharp from "sharp";

import type {
	ChallengeKind,
	CourseAppPurpose,
	CourseAssetKind,
	CourseCategory,
	CourseDifficulty,
	LessonAppRefKind,
} from "./api-types";
import {
	type JsonValue,
	UNIVERSITY_PLAN_SCHEMA,
	type UniversityAppLinkPlan,
	type UniversityAppRefPlan,
	type UniversityAssetPlan,
	type UniversityChallengePlan,
	type UniversityCoursePlan,
	type UniversityLessonPlan,
	type UniversityMediaPlan,
	type UniversityModulePlan,
	type UniversityOperation,
	type UniversityPlan,
} from "./types";

const SAFE_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const ASSET_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_-]{0,63}$/;
const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const LANGUAGE_PATTERN = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/;
const MIME_PATTERN =
	/^[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]*\/[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]*$/;
const MAX_FILE_BYTES = 2_147_483_647;
const MAX_TEXT_BYTES = 1_500_000;
const MAX_LESSON_JSON_BYTES = 1_900_000;
const MAX_API_STRING_BYTES = 255;

const DIFFICULTIES = [
	"BEGINNER",
	"INTERMEDIATE",
	"ADVANCED",
	"EXPERT",
] as const satisfies readonly CourseDifficulty[];
const CATEGORIES = [
	"GENERAL",
	"GETTING_STARTED",
	"FLOWS",
	"PAGES",
	"EVENTS",
	"DATA",
	"AI",
	"INTEGRATIONS",
	"DEPLOYMENT",
	"ADVANCED",
	"EXPERT",
] as const satisfies readonly CourseCategory[];
const ASSET_KINDS = [
	"IMAGE",
	"VIDEO",
	"AUDIO",
	"DOCUMENT",
] as const satisfies readonly CourseAssetKind[];
const CHALLENGE_KINDS = [
	"SINGLE_CHOICE",
	"MULTIPLE_CHOICE",
	"BOARD_RIDDLE",
	"EXECUTE_NODE",
] as const satisfies readonly ChallengeKind[];
const APP_PURPOSES = [
	"SHARED_TEMPLATE",
	"REFERENCE",
	"PLAYGROUND",
] as const satisfies readonly CourseAppPurpose[];
const APP_REF_KINDS = [
	"NAVIGATE",
	"FOCUS_NODE",
	"ADD_NODE",
	"CREATE_EVENT",
	"OPEN_OR_CLONE_APP",
] as const satisfies readonly LessonAppRefKind[];
const NAVIGATE_SUBPATHS = ["config", "events", "pages", "flow", "use"] as const;
const BOARD_PREDICATE_OPS = [
	"requires_nodes",
	"forbids_nodes",
	"max_nodes",
	"min_nodes",
	"has_connection",
	"pin_value_equals",
] as const;

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
	avif: "image/avif",
	bmp: "image/bmp",
	csv: "text/csv",
	gif: "image/gif",
	htm: "text/html",
	html: "text/html",
	jpeg: "image/jpeg",
	jpg: "image/jpeg",
	json: "application/json",
	md: "text/markdown",
	markdown: "text/markdown",
	mov: "video/quicktime",
	mp3: "audio/mpeg",
	mp4: "video/mp4",
	ogg: "audio/ogg",
	pdf: "application/pdf",
	png: "image/png",
	ppt: "application/vnd.ms-powerpoint",
	pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
	svg: "image/svg+xml",
	txt: "text/plain",
	wav: "audio/wav",
	webm: "video/webm",
	webp: "image/webp",
	xls: "application/vnd.ms-excel",
	xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	zip: "application/zip",
};

function record(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error(`${label} must be an object.`);
	}
	return value as Record<string, unknown>;
}

function rejectUnknownKeys(
	input: Record<string, unknown>,
	label: string,
	allowedKeys: readonly string[],
): void {
	const allowed = new Set(allowedKeys);
	for (const key of Object.keys(input)) {
		if (!allowed.has(key)) throw new Error(`${label}.${key} is not supported.`);
	}
}

function stringValue(value: unknown, label: string): string {
	if (typeof value !== "string" || value.trim().length === 0) {
		throw new Error(`${label} must be a non-empty string.`);
	}
	return value;
}

function nullableString(
	value: unknown,
	label: string,
	fallback: string | null = null,
): string | null {
	if (value === undefined || value === null) return fallback;
	return stringValue(value, label);
}

function utf8ByteLength(value: string): number {
	return new TextEncoder().encode(value).byteLength;
}

function validateContentSize(content: string, label: string): void {
	if (utf8ByteLength(content) > MAX_TEXT_BYTES) {
		throw new Error(`${label} exceeds ${MAX_TEXT_BYTES} bytes.`);
	}
	if (utf8ByteLength(JSON.stringify({ content })) > MAX_LESSON_JSON_BYTES) {
		throw new Error(
			`${label} expands beyond the safe University JSON request size of ${MAX_LESSON_JSON_BYTES} bytes.`,
		);
	}
}

function languageValue(value: unknown, label: string): string {
	const language = stringValue(value, label);
	if (!LANGUAGE_PATTERN.test(language)) {
		throw new Error(`${label} must be a BCP 47-style language tag.`);
	}
	return language;
}

function nullableHttpUrl(value: unknown, label: string): string | null {
	const input = nullableString(value, label);
	if (input === null) return null;
	const raw = input.trim();
	let url: URL;
	try {
		url = new URL(raw);
	} catch {
		throw new Error(`${label} must be an absolute HTTP or HTTPS URL.`);
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") {
		throw new Error(`${label} must be an absolute HTTP or HTTPS URL.`);
	}
	if (url.username || url.password) {
		throw new Error(`${label} cannot contain credentials.`);
	}
	return raw;
}

function unsupportedCourseMediaUrl(
	value: unknown,
	label: string,
	mediaField: "icon" | "banner",
): null {
	if (value !== undefined && value !== null) {
		throw new Error(
			`${label} is not supported by the University API; use plan.course.media.${mediaField} with a local image file instead.`,
		);
	}
	return null;
}

function aliasValue(value: unknown, label: string): string {
	const alias = stringValue(value, label).trim();
	if (!ASSET_NAME_PATTERN.test(alias)) {
		throw new Error(
			`${label} must start with a letter or underscore and contain at most 64 letters, digits, underscores, or dashes.`,
		);
	}
	return alias;
}

function booleanValue(
	value: unknown,
	label: string,
	fallback: boolean,
): boolean {
	if (value === undefined) return fallback;
	if (typeof value !== "boolean") throw new Error(`${label} must be boolean.`);
	return value;
}

function integerValue(
	value: unknown,
	label: string,
	fallback: number,
	min = 0,
	max = 2_147_483_647,
): number {
	if (value === undefined) return fallback;
	if (
		!Number.isSafeInteger(value) ||
		(value as number) < min ||
		(value as number) > max
	) {
		throw new Error(`${label} must be an integer from ${min} to ${max}.`);
	}
	return value as number;
}

function nullablePosition(value: unknown, label: string): number | null {
	if (value === undefined || value === null) return null;
	return integerValue(value, label, 0);
}

function enumValue<T extends string>(
	value: unknown,
	label: string,
	values: readonly T[],
	fallback?: T,
): T {
	if (value === undefined && fallback !== undefined) return fallback;
	if (typeof value !== "string" || !values.includes(value as T)) {
		throw new Error(`${label} must be one of: ${values.join(", ")}.`);
	}
	return value as T;
}

function idValue(value: unknown, label: string, fallback: string): string {
	const id = value === undefined ? fallback : stringValue(value, label).trim();
	if (!SAFE_ID_PATTERN.test(id)) {
		throw new Error(
			`${label} must start with an alphanumeric character, contain at most 128 letters, digits, dots, underscores, or dashes, and contain no path separators.`,
		);
	}
	return id;
}

function slugPart(value: string): string {
	const slug = value
		.normalize("NFKD")
		.replace(/\p{M}/gu, "")
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "")
		.slice(0, 48)
		.replace(/-+$/g, "");
	return slug || "item";
}

function derivedId(
	parent: string,
	kind: string,
	position: number,
	name: string,
): string {
	const suffix = slugPart(name);
	const candidate = `${parent}.${kind}.${position}-${suffix}`;
	if (candidate.length <= 128) return candidate;
	return `${parent.slice(0, Math.max(1, 117 - suffix.length))}.${kind}.${position}-${suffix}`.slice(
		0,
		128,
	);
}

function jsonValue(value: unknown, label: string): JsonValue {
	if (
		value === null ||
		typeof value === "string" ||
		typeof value === "boolean" ||
		(typeof value === "number" && Number.isFinite(value))
	)
		return value;
	if (Array.isArray(value))
		return value.map((item, index) => jsonValue(item, `${label}[${index}]`));
	if (value && typeof value === "object") {
		const output: Record<string, JsonValue> = {};
		for (const [key, item] of Object.entries(value)) {
			if (!key) throw new Error(`${label} cannot contain an empty key.`);
			output[key] = jsonValue(item, `${label}.${key}`);
		}
		return output;
	}
	throw new Error(`${label} must contain only JSON values.`);
}

function arrayValue(
	value: unknown,
	label: string,
	required = false,
): unknown[] {
	if (value === undefined && !required) return [];
	if (!Array.isArray(value) || (required && value.length === 0)) {
		throw new Error(
			`${label} must be ${required ? "a non-empty" : "an"} array.`,
		);
	}
	return value;
}

function stringArray(value: unknown, label: string): string[] {
	const values = arrayValue(value, label);
	const output = values.map((item, index) =>
		stringValue(item, `${label}[${index}]`).trim(),
	);
	assertUnique(output, label);
	return output;
}

function assertUnique(values: readonly string[], label: string): void {
	const seen = new Set<string>();
	for (const value of values) {
		if (seen.has(value))
			throw new Error(`${label} contains duplicate value ${value}.`);
		seen.add(value);
	}
}

function claimGlobalId(
	id: string,
	label: string,
	ids: Map<string, string>,
): void {
	const previous = ids.get(id);
	if (previous)
		throw new Error(
			`Duplicate id ${id} at ${label}; already used by ${previous}.`,
		);
	ids.set(id, label);
}

function claimPosition(
	position: number,
	label: string,
	positions: Set<number>,
): void {
	if (positions.has(position))
		throw new Error(`Duplicate position ${position} in ${label}.`);
	positions.add(position);
}

function resolvePlanPath(path: string, planPath?: string): string {
	if (isAbsolute(path)) return normalize(path);
	const base = planPath ? dirname(resolve(planPath)) : process.cwd();
	return resolve(base, path);
}

async function checkedFile(path: string, label: string): Promise<number> {
	let info: Awaited<ReturnType<typeof stat>>;
	try {
		info = await stat(path);
	} catch (error) {
		throw new Error(`${label} could not be read at ${path}: ${String(error)}`);
	}
	if (!info.isFile())
		throw new Error(`${label} must reference a regular file: ${path}.`);
	if (info.size > MAX_FILE_BYTES)
		throw new Error(`${label} exceeds ${MAX_FILE_BYTES} bytes.`);
	return info.size;
}

function extensionFor(path: string, label: string): string {
	const extension = extname(path).slice(1).toLowerCase();
	if (!extension || extension.length > 10 || !/^[a-z0-9]+$/.test(extension)) {
		throw new Error(
			`${label} must have an alphanumeric file extension of at most 10 characters.`,
		);
	}
	return extension;
}

function inferredMimeType(path: string, label: string): string {
	const extension = extensionFor(path, label);
	return MIME_BY_EXTENSION[extension] ?? "application/octet-stream";
}

function inferredAssetKind(mimeType: string): CourseAssetKind {
	if (mimeType.startsWith("image/")) return "IMAGE";
	if (mimeType.startsWith("video/")) return "VIDEO";
	if (mimeType.startsWith("audio/")) return "AUDIO";
	return "DOCUMENT";
}

function validateMimeKind(
	kind: CourseAssetKind,
	mimeType: string,
	label: string,
): void {
	if (!MIME_PATTERN.test(mimeType))
		throw new Error(`${label}.mimeType is not a valid MIME type.`);
	if (kind !== "DOCUMENT" && !mimeType.startsWith(`${kind.toLowerCase()}/`)) {
		throw new Error(
			`${label}.kind ${kind} does not match MIME type ${mimeType}.`,
		);
	}
}

function targetSelector(
	payload: Record<string, unknown>,
	label: string,
	requireApp: boolean,
	requireBoard: boolean,
): { appAlias?: string; appId?: string; boardId?: string } {
	const appAlias =
		payload.appAlias === undefined
			? undefined
			: aliasValue(payload.appAlias, `${label}.appAlias`);
	const appId =
		payload.appId === undefined
			? undefined
			: idValue(payload.appId, `${label}.appId`, "");
	const boardId =
		payload.boardId === undefined
			? undefined
			: idValue(payload.boardId, `${label}.boardId`, "");
	if (appAlias !== undefined && appId !== undefined) {
		throw new Error(`${label} cannot contain both appAlias and appId.`);
	}
	if (requireApp && appAlias === undefined && appId === undefined) {
		throw new Error(`${label} requires appAlias or appId.`);
	}
	if (requireBoard && boardId === undefined)
		throw new Error(`${label}.boardId is required.`);
	return { appAlias, appId, boardId };
}

function validateChoicePayload(
	value: unknown,
	label: string,
	multiple: boolean,
): JsonValue {
	const input = record(value, label);
	rejectUnknownKeys(input, label, ["options", "correct"]);
	const optionsInput = arrayValue(input.options, `${label}.options`, true);
	if (optionsInput.length < 2)
		throw new Error(`${label}.options must contain at least two options.`);
	const optionIds = new Set<string>();
	const options = optionsInput.map((value, index) => {
		const optionLabel = `${label}.options[${index}]`;
		const option = record(value, optionLabel);
		rejectUnknownKeys(option, optionLabel, ["id", "label"]);
		const id = idValue(option.id, `${optionLabel}.id`, `option-${index + 1}`);
		if (optionIds.has(id))
			throw new Error(`Duplicate choice option id ${id} in ${label}.`);
		optionIds.add(id);
		return { id, label: stringValue(option.label, `${optionLabel}.label`) };
	});
	const correct = stringArray(input.correct, `${label}.correct`);
	if (correct.length === 0)
		throw new Error(`${label}.correct must not be empty.`);
	if (!multiple && correct.length !== 1)
		throw new Error(`${label}.correct must contain exactly one option id.`);
	for (const id of correct) {
		if (!optionIds.has(id))
			throw new Error(`${label}.correct references unknown option id ${id}.`);
	}
	return { options, correct };
}

function validateBoardRiddlePayload(value: unknown, label: string): JsonValue {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"appAlias",
		"appId",
		"boardId",
		"predicates",
	]);
	const target = targetSelector(input, label, true, true);
	const predicatesInput = arrayValue(
		input.predicates,
		`${label}.predicates`,
		true,
	);
	const requiredNodeTypes = new Set<string>();
	const forbiddenNodeTypes = new Set<string>();
	let minimumNodes: number | undefined;
	let maximumNodes: number | undefined;
	const predicates = predicatesInput.map((value, index) => {
		const predicateLabel = `${label}.predicates[${index}]`;
		const predicate = record(value, predicateLabel);
		rejectUnknownKeys(predicate, predicateLabel, ["op", "args"]);
		const op = enumValue(
			predicate.op,
			`${predicateLabel}.op`,
			BOARD_PREDICATE_OPS,
		);
		const args = arrayValue(predicate.args, `${predicateLabel}.args`, true).map(
			(arg, argIndex) => jsonValue(arg, `${predicateLabel}.args[${argIndex}]`),
		);
		if (
			(op === "requires_nodes" || op === "forbids_nodes") &&
			!args.every((arg) => typeof arg === "string" && arg.trim())
		) {
			throw new Error(
				`${predicateLabel}.args must contain non-empty node type strings.`,
			);
		}
		if (op === "requires_nodes") {
			for (const arg of args) requiredNodeTypes.add(arg as string);
		}
		if (op === "forbids_nodes") {
			for (const arg of args) forbiddenNodeTypes.add(arg as string);
		}
		if (
			(op === "max_nodes" || op === "min_nodes") &&
			(args.length !== 1 ||
				!Number.isSafeInteger(args[0]) ||
				(args[0] as number) < 0)
		) {
			throw new Error(
				`${predicateLabel}.args must contain one non-negative integer.`,
			);
		}
		if (op === "max_nodes") maximumNodes = args[0] as number;
		if (op === "min_nodes") minimumNodes = args[0] as number;
		if (
			op === "has_connection" &&
			(args.length !== 2 ||
				!args.every((arg) => typeof arg === "string" && arg.trim()))
		) {
			throw new Error(
				`${predicateLabel}.args must contain source and target node type strings.`,
			);
		}
		if (
			op === "pin_value_equals" &&
			(args.length !== 3 ||
				typeof args[0] !== "string" ||
				!args[0].trim() ||
				typeof args[1] !== "string" ||
				!args[1].trim())
		) {
			throw new Error(
				`${predicateLabel}.args must contain node type, pin name, and expected JSON value.`,
			);
		}
		return { op, args };
	});
	for (const nodeType of requiredNodeTypes) {
		if (forbiddenNodeTypes.has(nodeType)) {
			throw new Error(
				`${label} cannot both require and forbid node type ${nodeType}.`,
			);
		}
	}
	if (
		minimumNodes !== undefined &&
		maximumNodes !== undefined &&
		minimumNodes > maximumNodes
	) {
		throw new Error(`${label} cannot set min_nodes above max_nodes.`);
	}
	return {
		...(target.appAlias === undefined ? {} : { appAlias: target.appAlias }),
		...(target.appId === undefined ? {} : { appId: target.appId }),
		boardId: target.boardId as string,
		predicates,
	};
}

function validateExecuteNodePayload(value: unknown, label: string): JsonValue {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"appAlias",
		"appId",
		"boardId",
		"nodeId",
		"requiredPackages",
	]);
	const target = targetSelector(input, label, true, true);
	const nodeId = idValue(input.nodeId, `${label}.nodeId`, "");
	const requiredPackages = stringArray(
		input.requiredPackages,
		`${label}.requiredPackages`,
	);
	if (requiredPackages.length === 0)
		throw new Error(`${label}.requiredPackages must not be empty.`);
	return {
		...(target.appAlias === undefined ? {} : { appAlias: target.appAlias }),
		...(target.appId === undefined ? {} : { appId: target.appId }),
		boardId: target.boardId as string,
		nodeId,
		requiredPackages,
	};
}

function challengePayload(
	kind: ChallengeKind,
	value: unknown,
	label: string,
): JsonValue {
	if (kind === "SINGLE_CHOICE")
		return validateChoicePayload(value, label, false);
	if (kind === "MULTIPLE_CHOICE")
		return validateChoicePayload(value, label, true);
	if (kind === "BOARD_RIDDLE") return validateBoardRiddlePayload(value, label);
	return validateExecuteNodePayload(value, label);
}

function validateAppRefTarget(
	kind: LessonAppRefKind,
	value: unknown,
	label: string,
): JsonValue {
	const input = record(value, label);
	const shapes: Record<LessonAppRefKind, readonly string[]> = {
		NAVIGATE: ["subpath", "params"],
		FOCUS_NODE: ["boardId", "nodeId"],
		ADD_NODE: ["boardId", "nodeTypeId", "coords"],
		CREATE_EVENT: ["template"],
		OPEN_OR_CLONE_APP: ["sharedAppId"],
	};
	rejectUnknownKeys(input, label, shapes[kind]);
	if (kind === "NAVIGATE") {
		const subpath = enumValue(
			input.subpath,
			`${label}.subpath`,
			NAVIGATE_SUBPATHS,
		);
		let params: Record<string, JsonValue> | undefined;
		if (input.params !== undefined) {
			params = {};
			for (const [key, value] of Object.entries(
				record(input.params, `${label}.params`),
			)) {
				if (!key || typeof value !== "string")
					throw new Error(
						`${label}.params must contain non-empty keys and string values.`,
					);
				params[key] = value;
			}
		}
		return { subpath, ...(params ? { params } : {}) };
	}
	if (kind === "FOCUS_NODE")
		return {
			boardId: idValue(input.boardId, `${label}.boardId`, ""),
			nodeId: idValue(input.nodeId, `${label}.nodeId`, ""),
		};
	if (kind === "ADD_NODE") {
		let coords: JsonValue | undefined;
		if (input.coords !== undefined) {
			if (
				!Array.isArray(input.coords) ||
				input.coords.length !== 2 ||
				!input.coords.every(
					(item) => typeof item === "number" && Number.isFinite(item),
				)
			)
				throw new Error(`${label}.coords must be a finite [x, y] tuple.`);
			coords = input.coords as [number, number];
		}
		return {
			boardId: idValue(input.boardId, `${label}.boardId`, ""),
			nodeTypeId: stringValue(input.nodeTypeId, `${label}.nodeTypeId`),
			...(coords ? { coords } : {}),
		};
	}
	if (kind === "CREATE_EVENT")
		return {
			template: jsonValue(
				record(input.template, `${label}.template`),
				`${label}.template`,
			),
		};
	return {
		sharedAppId: idValue(input.sharedAppId, `${label}.sharedAppId`, ""),
	};
}

interface ValidationContext {
	planPath?: string;
	ids: Map<string, string>;
	files: Array<{
		path: string;
		label: string;
		setSize?: (size: number) => void;
	}>;
	contentFiles: Array<{
		path: string;
		label: string;
		lesson: UniversityLessonPlan;
	}>;
}

function validateChallenge(
	value: unknown,
	label: string,
	lessonId: string,
	index: number,
	positions: Set<number>,
	context: ValidationContext,
): UniversityChallengePlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"id",
		"kind",
		"prompt",
		"explanation",
		"payload",
		"points",
		"position",
	]);
	const prompt = stringValue(input.prompt, `${label}.prompt`);
	const position = integerValue(input.position, `${label}.position`, index);
	claimPosition(position, `${label}.position`, positions);
	const id = idValue(
		input.id,
		`${label}.id`,
		derivedId(lessonId, "challenge", position, prompt),
	);
	claimGlobalId(id, `${label}.id`, context.ids);
	const kind = enumValue(input.kind, `${label}.kind`, CHALLENGE_KINDS);
	return {
		id,
		kind,
		prompt,
		explanation: nullableString(input.explanation, `${label}.explanation`),
		payload: challengePayload(kind, input.payload, `${label}.payload`),
		points: integerValue(input.points, `${label}.points`, 10),
		position,
	};
}

function validateAppRef(
	value: unknown,
	label: string,
	lessonId: string,
	index: number,
	context: ValidationContext,
): UniversityAppRefPlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"id",
		"kind",
		"target",
		"appAlias",
		"appId",
		"label",
	]);
	const kind = enumValue(input.kind, `${label}.kind`, APP_REF_KINDS);
	if (kind === "OPEN_OR_CLONE_APP") {
		throw new Error(
			`${label}.kind OPEN_OR_CLONE_APP is not supported by this plan schema; use NAVIGATE with appAlias and target.subpath "use" to open or clone a course app.`,
		);
	}
	const displayLabel = nullableString(input.label, `${label}.label`);
	const id = idValue(
		input.id,
		`${label}.id`,
		derivedId(lessonId, "ref", index, displayLabel ?? kind),
	);
	claimGlobalId(id, `${label}.id`, context.ids);
	const appAlias =
		input.appAlias === undefined || input.appAlias === null
			? null
			: aliasValue(input.appAlias, `${label}.appAlias`);
	const appId =
		input.appId === undefined || input.appId === null
			? null
			: idValue(input.appId, `${label}.appId`, "");
	if (appAlias && appId)
		throw new Error(`${label} cannot contain both appAlias and appId.`);
	if (!appAlias && !appId) {
		throw new Error(`${label} requires appAlias or appId.`);
	}
	return {
		id,
		kind,
		target: validateAppRefTarget(kind, input.target, `${label}.target`),
		appAlias,
		appId,
		label: displayLabel,
	};
}

function validateLesson(
	value: unknown,
	label: string,
	moduleId: string,
	index: number,
	courseLanguage: string,
	positions: Set<number>,
	context: ValidationContext,
): UniversityLessonPlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"id",
		"title",
		"language",
		"content",
		"contentFile",
		"videoUrl",
		"estimatedMinutes",
		"position",
		"isOptional",
		"finalAssessment",
		"challenges",
		"appRefs",
	]);
	const title = stringValue(input.title, `${label}.title`);
	const position = integerValue(input.position, `${label}.position`, index);
	claimPosition(position, `${label}.position`, positions);
	const id = idValue(
		input.id,
		`${label}.id`,
		derivedId(moduleId, "lesson", position, title),
	);
	claimGlobalId(id, `${label}.id`, context.ids);
	const hasContent = input.content !== undefined;
	const hasContentFile = input.contentFile !== undefined;
	if (hasContent === hasContentFile)
		throw new Error(`${label} requires exactly one of content or contentFile.`);
	const content = hasContent
		? stringValue(input.content, `${label}.content`)
		: "";
	if (hasContent) validateContentSize(content, `${label}.content`);
	let contentFile: string | undefined;
	const lesson: UniversityLessonPlan = {
		id,
		title,
		language: languageValue(
			input.language ?? courseLanguage,
			`${label}.language`,
		),
		content,
		videoUrl: nullableHttpUrl(input.videoUrl, `${label}.videoUrl`),
		estimatedMinutes: integerValue(
			input.estimatedMinutes,
			`${label}.estimatedMinutes`,
			5,
		),
		position,
		isOptional: booleanValue(input.isOptional, `${label}.isOptional`, false),
		finalAssessment: booleanValue(
			input.finalAssessment,
			`${label}.finalAssessment`,
			false,
		),
		challenges: [],
		appRefs: [],
	};
	if (hasContentFile) {
		contentFile = resolvePlanPath(
			stringValue(input.contentFile, `${label}.contentFile`),
			context.planPath,
		);
		lesson.contentFile = contentFile;
		context.files.push({ path: contentFile, label: `${label}.contentFile` });
		context.contentFiles.push({
			path: contentFile,
			label: `${label}.contentFile`,
			lesson,
		});
	}
	const challengePositions = new Set<number>();
	lesson.challenges = arrayValue(input.challenges, `${label}.challenges`).map(
		(challenge, challengeIndex) =>
			validateChallenge(
				challenge,
				`${label}.challenges[${challengeIndex}]`,
				id,
				challengeIndex,
				challengePositions,
				context,
			),
	);
	lesson.appRefs = arrayValue(input.appRefs, `${label}.appRefs`).map(
		(appRef, appRefIndex) =>
			validateAppRef(
				appRef,
				`${label}.appRefs[${appRefIndex}]`,
				id,
				appRefIndex,
				context,
			),
	);
	return lesson;
}

function validateModule(
	value: unknown,
	label: string,
	courseId: string,
	index: number,
	courseLanguage: string,
	positions: Set<number>,
	context: ValidationContext,
): UniversityModulePlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"id",
		"title",
		"description",
		"position",
		"lessons",
	]);
	const title = stringValue(input.title, `${label}.title`);
	const position = integerValue(input.position, `${label}.position`, index);
	claimPosition(position, `${label}.position`, positions);
	const id = idValue(
		input.id,
		`${label}.id`,
		derivedId(courseId, "module", position, title),
	);
	claimGlobalId(id, `${label}.id`, context.ids);
	const lessonPositions = new Set<number>();
	const lessonTitles = new Set<string>();
	return {
		id,
		title,
		description: nullableString(input.description, `${label}.description`),
		position,
		lessons: arrayValue(input.lessons, `${label}.lessons`, true).map(
			(lesson, lessonIndex) => {
				const parsed = validateLesson(
					lesson,
					`${label}.lessons[${lessonIndex}]`,
					id,
					lessonIndex,
					courseLanguage,
					lessonPositions,
					context,
				);
				if (lessonTitles.has(parsed.title)) {
					throw new Error(
						`Duplicate lesson title ${parsed.title} in ${label}.`,
					);
				}
				lessonTitles.add(parsed.title);
				return parsed;
			},
		),
	};
}

function validateAsset(
	value: unknown,
	label: string,
	context: ValidationContext,
): UniversityAssetPlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"name",
		"file",
		"kind",
		"mimeType",
		"filename",
		"replace",
	]);
	const name = stringValue(input.name, `${label}.name`).trim();
	if (!ASSET_NAME_PATTERN.test(name))
		throw new Error(
			`${label}.name must start with a letter or underscore and contain at most 64 letters, digits, underscores, or dashes.`,
		);
	const file = resolvePlanPath(
		stringValue(input.file, `${label}.file`),
		context.planPath,
	);
	const extension = extensionFor(file, `${label}.file`);
	const mimeType =
		input.mimeType === undefined
			? inferredMimeType(file, `${label}.file`)
			: stringValue(input.mimeType, `${label}.mimeType`).trim().toLowerCase();
	const kind = enumValue(
		input.kind,
		`${label}.kind`,
		ASSET_KINDS,
		inferredAssetKind(mimeType),
	);
	validateMimeKind(kind, mimeType, label);
	const filename =
		input.filename === undefined
			? basename(file)
			: stringValue(input.filename, `${label}.filename`).trim();
	if (basename(filename) !== filename || filename === "." || filename === "..")
		throw new Error(
			`${label}.filename must be a file name without directory components.`,
		);
	if (utf8ByteLength(filename) > MAX_API_STRING_BYTES) {
		throw new Error(
			`${label}.filename cannot exceed ${MAX_API_STRING_BYTES} UTF-8 bytes.`,
		);
	}
	if (utf8ByteLength(mimeType) > MAX_API_STRING_BYTES) {
		throw new Error(
			`${label}.mimeType cannot exceed ${MAX_API_STRING_BYTES} bytes.`,
		);
	}
	const output: UniversityAssetPlan = {
		name,
		file,
		kind,
		mimeType,
		filename,
		replace: booleanValue(input.replace, `${label}.replace`, false),
		size: 0,
		extension,
	};
	context.files.push({
		path: file,
		label: `${label}.file`,
		setSize: (size) => {
			output.size = size;
		},
	});
	return output;
}

function validateMedia(
	value: unknown,
	label: string,
	context: ValidationContext,
): UniversityMediaPlan | undefined {
	if (value === undefined) return undefined;
	const input = record(value, label);
	rejectUnknownKeys(input, label, ["icon", "banner"]);
	const output: UniversityMediaPlan = {};
	for (const item of ["icon", "banner"] as const) {
		if (input[item] === undefined) continue;
		const path = resolvePlanPath(
			stringValue(input[item], `${label}.${item}`),
			context.planPath,
		);
		extensionFor(path, `${label}.${item}`);
		const mimeType = inferredMimeType(path, `${label}.${item}`);
		if (!mimeType.startsWith("image/")) {
			throw new Error(`${label}.${item} must reference an image file.`);
		}
		output[item] = path;
		context.files.push({ path, label: `${label}.${item}` });
	}
	if (!output.icon && !output.banner)
		throw new Error(`${label} must contain icon or banner.`);
	return output;
}

function validateAppLink(
	value: unknown,
	label: string,
	courseId: string,
	index: number,
	context: ValidationContext,
): UniversityAppLinkPlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, ["id", "appId", "purpose", "alias"]);
	const appId = idValue(input.appId, `${label}.appId`, "");
	const alias =
		input.alias === undefined || input.alias === null
			? null
			: aliasValue(input.alias, `${label}.alias`);
	const id = idValue(
		input.id,
		`${label}.id`,
		derivedId(courseId, "app-link", index, alias ?? appId),
	);
	claimGlobalId(id, `${label}.id`, context.ids);
	return {
		id,
		appId,
		purpose: enumValue(
			input.purpose,
			`${label}.purpose`,
			APP_PURPOSES,
			"SHARED_TEMPLATE",
		),
		alias,
	};
}

function validateCourse(
	value: unknown,
	label: string,
	context: ValidationContext,
): UniversityCoursePlan {
	const input = record(value, label);
	rejectUnknownKeys(input, label, [
		"id",
		"name",
		"language",
		"slug",
		"difficulty",
		"category",
		"estimatedMinutes",
		"isPublished",
		"iconUrl",
		"bannerUrl",
		"tags",
		"position",
		"description",
		"longDescription",
		"media",
		"assets",
		"appLinks",
		"modules",
	]);
	const name = stringValue(input.name, `${label}.name`);
	const id = idValue(input.id, `${label}.id`, slugPart(name));
	claimGlobalId(id, `${label}.id`, context.ids);
	const language = languageValue(input.language ?? "en", `${label}.language`);
	const slug = nullableString(input.slug, `${label}.slug`);
	if (slug && !SLUG_PATTERN.test(slug))
		throw new Error(
			`${label}.slug must contain lowercase letters and digits separated by single dashes.`,
		);
	const assetNames = new Set<string>();
	const assets = arrayValue(input.assets, `${label}.assets`).map(
		(asset, index) => {
			const parsed = validateAsset(asset, `${label}.assets[${index}]`, context);
			if (assetNames.has(parsed.name))
				throw new Error(`Duplicate asset name ${parsed.name}.`);
			assetNames.add(parsed.name);
			return parsed;
		},
	);
	const appLinkAliases = new Set<string>();
	const appIds = new Set<string>();
	const appLinks = arrayValue(input.appLinks, `${label}.appLinks`).map(
		(appLink, index) => {
			const parsed = validateAppLink(
				appLink,
				`${label}.appLinks[${index}]`,
				id,
				index,
				context,
			);
			if (appIds.has(parsed.appId))
				throw new Error(`Duplicate app link appId ${parsed.appId}.`);
			appIds.add(parsed.appId);
			if (parsed.alias) {
				if (appLinkAliases.has(parsed.alias))
					throw new Error(`Duplicate app link alias ${parsed.alias}.`);
				appLinkAliases.add(parsed.alias);
			}
			return parsed;
		},
	);
	const modulePositions = new Set<number>();
	const moduleTitles = new Set<string>();
	const modules = arrayValue(input.modules, `${label}.modules`, true).map(
		(module, index) => {
			const parsed = validateModule(
				module,
				`${label}.modules[${index}]`,
				id,
				index,
				language,
				modulePositions,
				context,
			);
			if (moduleTitles.has(parsed.title)) {
				throw new Error(`Duplicate module title ${parsed.title}.`);
			}
			moduleTitles.add(parsed.title);
			return parsed;
		},
	);
	return {
		id,
		name,
		language,
		slug,
		difficulty: enumValue(
			input.difficulty,
			`${label}.difficulty`,
			DIFFICULTIES,
			"BEGINNER",
		),
		category: enumValue(
			input.category,
			`${label}.category`,
			CATEGORIES,
			"GENERAL",
		),
		estimatedMinutes: integerValue(
			input.estimatedMinutes,
			`${label}.estimatedMinutes`,
			0,
		),
		isPublished: booleanValue(input.isPublished, `${label}.isPublished`, false),
		iconUrl: unsupportedCourseMediaUrl(
			input.iconUrl,
			`${label}.iconUrl`,
			"icon",
		),
		bannerUrl: unsupportedCourseMediaUrl(
			input.bannerUrl,
			`${label}.bannerUrl`,
			"banner",
		),
		tags: stringArray(input.tags, `${label}.tags`),
		position: nullablePosition(input.position, `${label}.position`),
		description: nullableString(input.description, `${label}.description`),
		longDescription: nullableString(
			input.longDescription,
			`${label}.longDescription`,
		),
		media: validateMedia(input.media, `${label}.media`, context),
		assets,
		appLinks,
		modules,
	};
}

function validateFinalAssessment(course: UniversityCoursePlan): void {
	const lessons = [...course.modules]
		.sort((left, right) => left.position - right.position)
		.flatMap((module) =>
			[...module.lessons].sort((left, right) => left.position - right.position),
		);
	const assessments = lessons.filter((lesson) => lesson.finalAssessment);
	if (assessments.length === 0) return;
	if (assessments.length !== 1)
		throw new Error("Only one lesson may set finalAssessment to true.");
	const assessment = assessments[0] as UniversityLessonPlan;
	if (lessons.at(-1) !== assessment)
		throw new Error(
			"The finalAssessment must be the last lesson by module and lesson position.",
		);
	if (assessment.isOptional)
		throw new Error(
			"The finalAssessment lesson must be required (isOptional false).",
		);
	if (assessment.challenges.length === 0)
		throw new Error(
			"The finalAssessment lesson must contain at least one challenge.",
		);
}

const PLACEHOLDER_LESSON_PATTERN =
	/\b(?:new lesson|placeholder lesson|start writing|coming soon|working hard to bring content)\b/i;
const EMBEDDED_SECRET_URL_PATTERN =
	/https?:\/\/\S+[?&][^\s=]*(?:token|key|secret|password|auth|code|signature|credential|sig|sas)[^\s=]*=/i;

function validatePublishedCourse(course: UniversityCoursePlan): void {
	if (!course.isPublished) return;
	if (course.estimatedMinutes <= 0) {
		throw new Error(
			"Published courses require estimatedMinutes greater than zero.",
		);
	}
	if (!course.description || !course.longDescription) {
		throw new Error(
			"Published courses require both description and longDescription.",
		);
	}
	const lessons = course.modules.flatMap((module) => module.lessons);
	const assetNames = new Set(course.assets.map((asset) => asset.name));
	if (!lessons.some((lesson) => lesson.finalAssessment)) {
		throw new Error(
			"Published courses require one finalAssessment lesson with challenges.",
		);
	}
	for (const lesson of lessons) {
		const content = lesson.content.trim();
		if (content.length < 160) {
			throw new Error(
				`Published lesson ${lesson.id} requires at least 160 characters of substantive content.`,
			);
		}
		if (PLACEHOLDER_LESSON_PATTERN.test(content)) {
			throw new Error(
				`Published lesson ${lesson.id} still contains placeholder content.`,
			);
		}
		if (EMBEDDED_SECRET_URL_PATTERN.test(content)) {
			throw new Error(
				`Published lesson ${lesson.id} contains a credential-bearing URL; upload it as a named course asset instead.`,
			);
		}
		for (const match of content.matchAll(
			/(^|[^\w\\])@([A-Za-z_][A-Za-z0-9_-]{0,63})/g,
		)) {
			const name = match[2] as string;
			if (!assetNames.has(name)) {
				throw new Error(
					`Published lesson ${lesson.id} references unknown course asset @${name}.`,
				);
			}
		}
	}
}

function validateAppAliasReferences(course: UniversityCoursePlan): void {
	const aliases = new Set(
		course.appLinks
			.map((appLink) => appLink.alias)
			.filter((alias): alias is string => alias !== null),
	);
	const openedAliases = new Set<string>();
	const modules = [...course.modules].sort(
		(left, right) => left.position - right.position,
	);
	for (const module of modules) {
		const lessons = [...module.lessons].sort(
			(left, right) => left.position - right.position,
		);
		for (const lesson of lessons) {
			for (const appRef of lesson.appRefs) {
				if (appRef.appAlias && !aliases.has(appRef.appAlias)) {
					throw new Error(
						`App reference ${appRef.id} references unknown appAlias ${appRef.appAlias}; declare it in plan.course.appLinks.`,
					);
				}
				if (appRef.appAlias) openedAliases.add(appRef.appAlias);
			}
			for (const challenge of lesson.challenges) {
				if (
					challenge.kind !== "BOARD_RIDDLE" &&
					challenge.kind !== "EXECUTE_NODE"
				) {
					continue;
				}
				const payload = challenge.payload as Record<string, JsonValue>;
				const alias = payload.appAlias;
				if (typeof alias === "string" && !aliases.has(alias)) {
					throw new Error(
						`Challenge ${challenge.id} references unknown appAlias ${alias}; declare it in plan.course.appLinks.`,
					);
				}
				if (typeof alias === "string" && !openedAliases.has(alias)) {
					throw new Error(
						`Challenge ${challenge.id} uses appAlias ${alias} before any appRef opens it; add an appRef with that alias in the same or an earlier lesson.`,
					);
				}
			}
		}
	}
}

function validateUniversityPlanShape(
	value: unknown,
	planPath?: string,
): { plan: UniversityPlan; context: ValidationContext } {
	const input = record(value, "plan");
	rejectUnknownKeys(input, "plan", ["schema", "course"]);
	if (input.schema !== UNIVERSITY_PLAN_SCHEMA)
		throw new Error(`plan.schema must be ${UNIVERSITY_PLAN_SCHEMA}.`);
	const context: ValidationContext = {
		planPath,
		ids: new Map(),
		files: [],
		contentFiles: [],
	};
	const course = validateCourse(input.course, "plan.course", context);
	validateFinalAssessment(course);
	validateAppAliasReferences(course);
	return { plan: { schema: UNIVERSITY_PLAN_SCHEMA, course }, context };
}

async function resolveLocalFiles(context: ValidationContext): Promise<void> {
	for (const file of context.files) {
		const size = await checkedFile(file.path, file.label);
		file.setSize?.(size);
	}
	for (const source of context.contentFiles) {
		const info = await stat(source.path);
		if (info.size > MAX_TEXT_BYTES)
			throw new Error(`${source.label} exceeds ${MAX_TEXT_BYTES} bytes.`);
		let content: string;
		try {
			content = new TextDecoder("utf-8", { fatal: true }).decode(
				await readFile(source.path),
			);
		} catch (error) {
			throw new Error(
				`${source.label} could not be read as UTF-8 at ${source.path}: ${String(error)}`,
			);
		}
		if (!content.trim()) throw new Error(`${source.label} must not be empty.`);
		validateContentSize(content, source.label);
		source.lesson.content = content;
	}
}

async function validateCourseMedia(
	course: UniversityCoursePlan,
): Promise<void> {
	for (const [item, path] of Object.entries(course.media ?? {})) {
		try {
			const metadata = await sharp(path).metadata();
			if (!metadata.width || !metadata.height) {
				throw new Error("image has no dimensions");
			}
		} catch (error) {
			throw new Error(
				`plan.course.media.${item} could not be decoded as an image at ${path}: ${String(error)}`,
			);
		}
	}
}

/**
 * Strictly validates and normalizes a plan. File paths are resolved relative to
 * `planPath` (or the current working directory), but filesystem access belongs
 * to `loadUniversityPlan`.
 */
export function validateUniversityPlan(
	value: unknown,
	planPath?: string,
): UniversityPlan {
	return validateUniversityPlanShape(value, planPath).plan;
}

/** Loads JSON, validates every local file, and materializes contentFile text. */
export async function loadUniversityPlan(
	path: string,
): Promise<UniversityPlan> {
	const planPath = resolve(path);
	let parsed: unknown;
	try {
		parsed = JSON.parse(await readFile(planPath, "utf8"));
	} catch (error) {
		throw new Error(
			`Could not read university plan ${planPath}: ${String(error)}`,
		);
	}
	const { plan, context } = validateUniversityPlanShape(parsed, planPath);
	await resolveLocalFiles(context);
	validatePublishedCourse(plan.course);
	await validateCourseMedia(plan.course);
	return plan;
}

/** Produces the deterministic, dependency-safe order used by apply and dry-run. */
export function buildUniversityOperations(
	plan: UniversityPlan,
): UniversityOperation[] {
	const operations: UniversityOperation[] = [
		{ type: "upsertCourse", course: plan.course, publish: false },
	];
	if (plan.course.media?.icon)
		operations.push({
			type: "uploadMedia",
			item: "icon",
			file: plan.course.media.icon,
			language: plan.course.language,
		});
	if (plan.course.media?.banner)
		operations.push({
			type: "uploadMedia",
			item: "banner",
			file: plan.course.media.banner,
			language: plan.course.language,
		});
	for (const asset of plan.course.assets)
		operations.push({ type: "uploadAsset", asset });
	for (const appLink of plan.course.appLinks)
		operations.push({ type: "upsertAppLink", appLink });
	for (const module of plan.course.modules) {
		operations.push({ type: "upsertModule", module });
		for (const lesson of module.lessons) {
			operations.push({ type: "upsertLesson", moduleId: module.id, lesson });
			for (const challenge of lesson.challenges)
				operations.push({
					type: "upsertChallenge",
					lessonId: lesson.id,
					challenge,
				});
			for (const appRef of lesson.appRefs)
				operations.push({ type: "upsertAppRef", lessonId: lesson.id, appRef });
		}
	}
	if (plan.course.isPublished)
		operations.push({
			type: "upsertCourse",
			course: plan.course,
			publish: true,
		});
	return operations;
}
