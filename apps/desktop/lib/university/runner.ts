import { readFile, stat } from "node:fs/promises";
import { basename, extname } from "node:path";
import sharp from "sharp";

import { type UniversityClient, createUniversityClient } from "./client";
import { buildUniversityOperations } from "./plan";
import type {
	UniversityAssetPlan,
	UniversityCoursePlan,
	UniversityOperation,
	UniversityOperationKind,
	UniversityPlan,
} from "./types";

export const UNIVERSITY_RESULT_SCHEMA =
	"flow-like.university-result/v1" as const;

export type UniversityCommand = "plan" | "apply" | "inspect" | "list" | "asset";
export type UniversityOperationStatus =
	| "planned"
	| "completed"
	| "skipped"
	| "failed";

export interface UniversityOperationResult {
	index: number;
	type: UniversityOperationKind | "verify" | "inspect" | "list";
	status: UniversityOperationStatus;
	description: string;
	durationMs?: number;
	error?: string;
}

export interface UniversityCommandResult {
	schema: typeof UNIVERSITY_RESULT_SCHEMA;
	command: UniversityCommand;
	passed: boolean;
	durationMs: number;
	courseId?: string;
	summary?: string;
	error?: string;
	operations?: UniversityOperationResult[];
	data?: unknown;
}

export interface UniversityRemoteOptions {
	apiUrl: string;
	pat: string;
	timeoutMs: number;
}

export interface UploadSingleUniversityAssetOptions {
	courseId: string;
	name: string;
	file: string;
	kind?: "IMAGE" | "VIDEO" | "AUDIO" | "DOCUMENT";
	mimeType?: string;
	replace: boolean;
}

interface ExecutionStep {
	operation?: UniversityOperation;
	verify?: true;
}

interface KnownAsset {
	id: string;
	name: string;
	filename: string;
	mimeType: string;
	size: number;
	kind: string;
	raw: unknown;
}

interface ApplyState {
	assets?: KnownAsset[];
}

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
	avif: "image/avif",
	bmp: "image/bmp",
	csv: "text/csv",
	gif: "image/gif",
	html: "text/html",
	jpeg: "image/jpeg",
	jpg: "image/jpeg",
	json: "application/json",
	md: "text/markdown",
	mov: "video/quicktime",
	mp3: "audio/mpeg",
	mp4: "video/mp4",
	ogg: "audio/ogg",
	pdf: "application/pdf",
	png: "image/png",
	svg: "image/svg+xml",
	txt: "text/plain",
	wav: "audio/wav",
	webm: "video/webm",
	webp: "image/webp",
	xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	zip: "application/zip",
};

function elapsed(startedAt: number): number {
	return Math.max(0, Date.now() - startedAt);
}

function errorMessage(error: unknown): string {
	const message = error instanceof Error ? error.message : String(error);
	return message
		.replace(/pat_[A-Za-z0-9._-]+/g, "[REDACTED]")
		.replace(
			/([?&][^=&\s]*(?:token|key|secret|password|passwd|auth|code|signature|credential|sig|sas)[^=&\s]*=)[^&\s]+/gi,
			"$1[REDACTED]",
		);
}

function record(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error(`${label} returned an unexpected response.`);
	}
	return value as Record<string, unknown>;
}

function array(value: unknown, label: string): unknown[] {
	if (!Array.isArray(value)) {
		throw new Error(`${label} returned an unexpected response.`);
	}
	return value;
}

function stringField(value: unknown, label: string): string {
	if (typeof value !== "string") {
		throw new Error(`${label} returned an unexpected response.`);
	}
	return value;
}

function numberField(value: unknown, label: string): number {
	if (typeof value !== "number" || !Number.isFinite(value)) {
		throw new Error(`${label} returned an unexpected response.`);
	}
	return value;
}

function courseBody(course: UniversityCoursePlan, publish: boolean) {
	return {
		language: course.language,
		slug: course.slug,
		difficulty: course.difficulty,
		category: course.category,
		estimated_minutes: course.estimatedMinutes,
		is_published: publish,
		icon_url: course.iconUrl,
		banner_url: course.bannerUrl,
		tags: course.tags,
		position: course.position,
		name: course.name,
		description: course.description,
		long_description: course.longDescription,
	};
}

function operationDescription(operation: UniversityOperation): string {
	switch (operation.type) {
		case "upsertCourse":
			return operation.publish
				? `Publish course ${operation.course.id}`
				: `Upsert draft course ${operation.course.id}`;
		case "uploadMedia":
			return `Upload course ${operation.item} from ${basename(operation.file)}`;
		case "uploadAsset":
			return `Upload asset @${operation.asset.name} from ${basename(operation.asset.file)}`;
		case "upsertAppLink":
			return `Upsert application link ${operation.appLink.id}`;
		case "upsertModule":
			return `Upsert module ${operation.module.id}`;
		case "upsertLesson":
			return `Upsert lesson ${operation.lesson.id}`;
		case "upsertChallenge":
			return `Upsert challenge ${operation.challenge.id}`;
		case "upsertAppRef":
			return `Upsert lesson application reference ${operation.appRef.id}`;
	}
}

function executionSteps(plan: UniversityPlan): ExecutionStep[] {
	const operations = buildUniversityOperations(plan);
	const finalPublish = operations.at(-1);
	if (finalPublish?.type === "upsertCourse" && finalPublish.publish) {
		return [
			...operations.slice(0, -1).map((operation) => ({ operation })),
			{ verify: true },
			{ operation: finalPublish },
		];
	}
	return [...operations.map((operation) => ({ operation })), { verify: true }];
}

function operationResults(
	steps: ExecutionStep[],
	status: UniversityOperationStatus,
): UniversityOperationResult[] {
	return steps.map((step, index) => ({
		index,
		type: step.verify ? "verify" : (step.operation?.type ?? "verify"),
		status,
		description: step.verify
			? "Verify the remote course structure and authored children"
			: operationDescription(step.operation as UniversityOperation),
	}));
}

function remoteRuntime(options: UniversityRemoteOptions): {
	client: UniversityClient;
	close: () => void;
} {
	const controller = new AbortController();
	const timeout = setTimeout(() => controller.abort(), options.timeoutMs);
	const client = createUniversityClient({
		baseUrl: options.apiUrl,
		pat: options.pat,
		signal: controller.signal,
	});
	return { client, close: () => clearTimeout(timeout) };
}

async function fileBlob(path: string, mimeType?: string): Promise<Blob> {
	if (typeof Bun !== "undefined") {
		return Bun.file(path, mimeType ? { type: mimeType } : undefined);
	}
	const bytes = Uint8Array.from(await readFile(path));
	return new Blob([bytes], mimeType ? { type: mimeType } : undefined);
}

async function courseMediaBlob(path: string): Promise<Blob> {
	if (extensionFor(path) === "webp") return fileBlob(path, "image/webp");
	const encoded = await sharp(path).webp({ quality: 85 }).toBuffer();
	return new Blob([Uint8Array.from(encoded)], { type: "image/webp" });
}

function extensionFor(path: string): string {
	const extension = extname(path).slice(1).toLowerCase();
	if (!extension || extension.length > 10 || !/^[a-z0-9]+$/.test(extension)) {
		throw new Error(
			`Asset file must have an alphanumeric extension of at most 10 characters: ${path}`,
		);
	}
	return extension;
}

function mimeTypeFor(path: string): string {
	return MIME_BY_EXTENSION[extensionFor(path)] ?? "application/octet-stream";
}

function assetKindFor(
	mimeType: string,
): "IMAGE" | "VIDEO" | "AUDIO" | "DOCUMENT" {
	if (mimeType.startsWith("image/")) return "IMAGE";
	if (mimeType.startsWith("video/")) return "VIDEO";
	if (mimeType.startsWith("audio/")) return "AUDIO";
	return "DOCUMENT";
}

function parseAsset(value: unknown): KnownAsset {
	const item = record(value, "Course asset");
	return {
		id: stringField(item.id, "Course asset id"),
		name: stringField(item.name, "Course asset name"),
		filename: stringField(item.filename, "Course asset filename"),
		mimeType: stringField(item.mime_type, "Course asset MIME type"),
		size: numberField(item.size, "Course asset size"),
		kind: stringField(item.kind, "Course asset kind"),
		raw: value,
	};
}

function assetMatches(
	existing: KnownAsset,
	wanted: UniversityAssetPlan,
): boolean {
	return (
		existing.name === wanted.name &&
		existing.filename === wanted.filename &&
		existing.mimeType === wanted.mimeType &&
		existing.size === wanted.size &&
		existing.kind === wanted.kind
	);
}

async function loadKnownAssets(
	client: UniversityClient,
	courseId: string,
): Promise<KnownAsset[]> {
	return array(
		await client.listCourseAssets(courseId),
		"Course asset list",
	).map(parseAsset);
}

async function applyAsset(
	client: UniversityClient,
	courseId: string,
	asset: UniversityAssetPlan,
	state: ApplyState,
): Promise<"completed" | "skipped"> {
	state.assets ??= await loadKnownAssets(client, courseId);
	const existing = state.assets.find((item) => item.name === asset.name);
	if (existing) {
		if (!asset.replace && assetMatches(existing, asset)) return "skipped";
		if (!asset.replace) {
			throw new Error(
				`Asset @${asset.name} already exists with different metadata. Set replace to true to replace it explicitly.`,
			);
		}
		await client.deleteCourseAsset(courseId, existing.id);
		state.assets = state.assets.filter((item) => item.id !== existing.id);
	}
	const uploaded = await client.uploadCourseAsset(
		courseId,
		{
			name: asset.name,
			filename: asset.filename,
			mime_type: asset.mimeType,
			size: asset.size,
			kind: asset.kind,
			extension: asset.extension,
		},
		await fileBlob(asset.file, asset.mimeType),
	);
	state.assets.push(parseAsset(uploaded));
	return "completed";
}

async function applyOperation(
	client: UniversityClient,
	courseId: string,
	operation: UniversityOperation,
	state: ApplyState,
): Promise<"completed" | "skipped"> {
	switch (operation.type) {
		case "upsertCourse": {
			const saved = await client.upsertCourse(
				courseId,
				courseBody(operation.course, operation.publish),
			);
			if (saved.id !== courseId || saved.is_published !== operation.publish) {
				throw new Error(
					`Course ${courseId} did not return the requested ${operation.publish ? "published" : "draft"} state.`,
				);
			}
			return "completed";
		}
		case "uploadMedia": {
			await client.uploadCourseMedia(
				courseId,
				{
					language: operation.language,
					item: operation.item === "icon" ? "icon" : "thumbnail",
					extension: "webp",
				},
				await courseMediaBlob(operation.file),
			);
			return "completed";
		}
		case "uploadAsset":
			return applyAsset(client, courseId, operation.asset, state);
		case "upsertAppLink":
			await client.upsertAppLink(courseId, operation.appLink.id, {
				app_id: operation.appLink.appId,
				purpose: operation.appLink.purpose,
				alias: operation.appLink.alias,
			});
			return "completed";
		case "upsertModule":
			await client.upsertModule(courseId, operation.module.id, {
				title: operation.module.title,
				description: operation.module.description,
				position: operation.module.position,
			});
			return "completed";
		case "upsertLesson":
			await client.upsertLesson(
				courseId,
				operation.moduleId,
				operation.lesson.id,
				{
					title: operation.lesson.title,
					language: operation.lesson.language,
					content: operation.lesson.content,
					video_url: operation.lesson.videoUrl,
					estimated_minutes: operation.lesson.estimatedMinutes,
					position: operation.lesson.position,
					is_optional: operation.lesson.isOptional,
				},
			);
			return "completed";
		case "upsertChallenge":
			await client.upsertChallenge(
				courseId,
				operation.lessonId,
				operation.challenge.id,
				{
					kind: operation.challenge.kind,
					prompt: operation.challenge.prompt,
					explanation: operation.challenge.explanation,
					payload: operation.challenge.payload,
					points: operation.challenge.points,
					position: operation.challenge.position,
				},
			);
			return "completed";
		case "upsertAppRef":
			await client.upsertAppRef(
				courseId,
				operation.lessonId,
				operation.appRef.id,
				{
					kind: operation.appRef.kind,
					target: operation.appRef.target,
					app_alias: operation.appRef.appAlias,
					app_id: operation.appRef.appId,
					label: operation.appRef.label,
				},
			);
			return "completed";
	}
}

function comparable(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(comparable).join(",")}]`;
	if (value && typeof value === "object") {
		const entries = Object.entries(value as Record<string, unknown>)
			.sort(([left], [right]) => left.localeCompare(right))
			.map(([key, item]) => `${JSON.stringify(key)}:${comparable(item)}`);
		return `{${entries.join(",")}}`;
	}
	return JSON.stringify(value);
}

function expectField(
	actual: Record<string, unknown>,
	field: string,
	expected: unknown,
	label: string,
	errors: string[],
): void {
	if (comparable(actual[field]) !== comparable(expected)) {
		errors.push(`${label}.${field} did not match the plan`);
	}
}

async function verifyRemotePlan(
	client: UniversityClient,
	plan: UniversityPlan,
): Promise<void> {
	const errors: string[] = [];
	const structure = record(
		await client.getCourseStructure(plan.course.id, plan.course.language),
		"Course structure",
	);
	const remoteCourse = record(structure.course, "Course structure course");
	for (const [field, expected] of [
		["id", plan.course.id],
		["language", plan.course.language],
		["slug", plan.course.slug],
		["difficulty", plan.course.difficulty],
		["category", plan.course.category],
		["estimated_minutes", plan.course.estimatedMinutes],
		["is_published", false],
		["tags", plan.course.tags],
		["position", plan.course.position],
		["name", plan.course.name],
		["description", plan.course.description],
		["long_description", plan.course.longDescription],
	] as const) {
		expectField(remoteCourse, field, expected, "course", errors);
	}
	if (plan.course.media?.icon && !remoteCourse.icon_url) {
		errors.push("course.icon_url was not set after the icon upload");
	}
	if (plan.course.media?.banner && !remoteCourse.banner_url) {
		errors.push("course.banner_url was not set after the banner upload");
	}

	const remoteModules = array(structure.modules, "Course modules").map((item) =>
		record(item, "Course module"),
	);
	const expectedModuleIds = new Set(
		plan.course.modules.map((module) => module.id),
	);
	for (const remote of remoteModules) {
		if (typeof remote.id === "string" && !expectedModuleIds.has(remote.id)) {
			errors.push(`unexpected module ${remote.id} remains on the course`);
		}
	}
	const lessonRequests: Array<Promise<unknown>> = [];
	const expectedLessons = plan.course.modules.flatMap((module) =>
		module.lessons.map((lesson) => ({ module, lesson })),
	);
	for (const module of plan.course.modules) {
		const remote = remoteModules.find((item) => item.id === module.id);
		if (!remote) {
			errors.push(`module ${module.id} was missing`);
			continue;
		}
		expectField(
			remote,
			"course_id",
			plan.course.id,
			`module ${module.id}`,
			errors,
		);
		expectField(remote, "title", module.title, `module ${module.id}`, errors);
		expectField(
			remote,
			"description",
			module.description,
			`module ${module.id}`,
			errors,
		);
		expectField(
			remote,
			"position",
			module.position,
			`module ${module.id}`,
			errors,
		);
		const summaries = Array.isArray(remote.lessons)
			? remote.lessons.map((item) => record(item, "Lesson summary"))
			: [];
		const expectedLessonIds = new Set(
			module.lessons.map((lesson) => lesson.id),
		);
		for (const summary of summaries) {
			if (
				typeof summary.id === "string" &&
				!expectedLessonIds.has(summary.id)
			) {
				errors.push(
					`unexpected lesson ${summary.id} remains in module ${module.id}`,
				);
			}
		}
		for (const lesson of module.lessons) {
			if (!summaries.some((item) => item.id === lesson.id)) {
				errors.push(`lesson ${lesson.id} was missing from module ${module.id}`);
			}
		}
	}
	for (const { module, lesson } of expectedLessons) {
		lessonRequests.push(client.getLesson(plan.course.id, module.id, lesson.id));
	}
	const lessonResponses = await Promise.all(lessonRequests);
	for (let index = 0; index < expectedLessons.length; index += 1) {
		const expected = expectedLessons[index];
		const response = record(lessonResponses[index], "Lesson detail");
		const remoteLesson = record(response.lesson, "Lesson");
		const label = `lesson ${expected.lesson.id}`;
		for (const [field, value] of [
			["id", expected.lesson.id],
			["module_id", expected.module.id],
			["title", expected.lesson.title],
			["language", expected.lesson.language],
			["content", expected.lesson.content],
			["video_url", expected.lesson.videoUrl],
			["estimated_minutes", expected.lesson.estimatedMinutes],
			["position", expected.lesson.position],
			["is_optional", expected.lesson.isOptional],
		] as const) {
			expectField(remoteLesson, field, value, label, errors);
		}
		const challenges = array(response.challenges, `${label} challenges`).map(
			(item) => record(item, "Challenge"),
		);
		const expectedChallengeIds = new Set(
			expected.lesson.challenges.map((challenge) => challenge.id),
		);
		for (const challenge of challenges) {
			if (
				typeof challenge.id === "string" &&
				!expectedChallengeIds.has(challenge.id)
			) {
				errors.push(`unexpected challenge ${challenge.id} remains in ${label}`);
			}
		}
		for (const challenge of expected.lesson.challenges) {
			const remote = challenges.find((item) => item.id === challenge.id);
			if (!remote) {
				errors.push(`challenge ${challenge.id} was missing from ${label}`);
				continue;
			}
			for (const [field, value] of [
				["lesson_id", expected.lesson.id],
				["kind", challenge.kind],
				["prompt", challenge.prompt],
				["explanation", challenge.explanation],
				["payload", challenge.payload],
				["points", challenge.points],
				["position", challenge.position],
			] as const) {
				expectField(remote, field, value, `challenge ${challenge.id}`, errors);
			}
		}
		const refs = array(
			response.app_refs,
			`${label} application references`,
		).map((item) => record(item, "Application reference"));
		const expectedAppRefIds = new Set(
			expected.lesson.appRefs.map((appRef) => appRef.id),
		);
		for (const appRef of refs) {
			if (typeof appRef.id === "string" && !expectedAppRefIds.has(appRef.id)) {
				errors.push(
					`unexpected application reference ${appRef.id} remains in ${label}`,
				);
			}
		}
		for (const appRef of expected.lesson.appRefs) {
			const remote = refs.find((item) => item.id === appRef.id);
			if (!remote) {
				errors.push(
					`application reference ${appRef.id} was missing from ${label}`,
				);
				continue;
			}
			for (const [field, value] of [
				["lesson_id", expected.lesson.id],
				["kind", appRef.kind],
				["target", appRef.target],
				["app_alias", appRef.appAlias],
				["app_id", appRef.appId],
				["label", appRef.label],
			] as const) {
				expectField(
					remote,
					field,
					value,
					`application reference ${appRef.id}`,
					errors,
				);
			}
		}
	}

	const lessonAssetViews = lessonResponses[0]
		? array(
				record(lessonResponses[0], "Lesson detail").assets,
				"Lesson asset views",
			).map((item) => record(item, "Lesson asset view"))
		: [];
	await Promise.all(
		plan.course.assets.map(async (wanted) => {
			const view = lessonAssetViews.find((item) => item.name === wanted.name);
			if (!view) {
				errors.push(`asset @${wanted.name} had no signed download URL`);
				return;
			}
			if (typeof view.signed_url !== "string" || !view.signed_url) {
				errors.push(`asset @${wanted.name} had no signed download URL`);
				return;
			}

			try {
				const response = await fetch(view.signed_url, {
					headers: { Range: "bytes=0-63" },
					redirect: "error",
					signal: AbortSignal.timeout(15_000),
				});
				if (!response.ok && response.status !== 206) {
					errors.push(
						`asset @${wanted.name} could not be downloaded (HTTP ${response.status})`,
					);
					return;
				}

				const contentType = response.headers
					.get("content-type")
					?.split(";", 1)[0]
					?.trim()
					.toLowerCase();
				if (contentType && contentType !== wanted.mimeType.toLowerCase()) {
					errors.push(
						`asset @${wanted.name} returned ${contentType} instead of ${wanted.mimeType}`,
					);
				}

				const reader = response.body?.getReader();
				if (!reader) {
					errors.push(`asset @${wanted.name} returned no file body`);
					return;
				}
				const firstChunk = await reader.read();
				await reader.cancel().catch(() => undefined);
				if (firstChunk.done || !firstChunk.value?.byteLength) {
					errors.push(`asset @${wanted.name} returned an empty file`);
				}
			} catch {
				errors.push(`asset @${wanted.name} could not be downloaded`);
			}
		}),
	);

	const assets = await loadKnownAssets(client, plan.course.id);
	for (const wanted of plan.course.assets) {
		const remote = assets.find((item) => item.name === wanted.name);
		if (!remote || !assetMatches(remote, wanted)) {
			errors.push(`asset @${wanted.name} did not match the plan`);
		}
	}
	const appLinks = array(
		await client.listAppLinks(plan.course.id),
		"Application links",
	).map((item) => record(item, "Application link"));
	for (const link of plan.course.appLinks) {
		const remote = appLinks.find((item) => item.id === link.id);
		if (!remote) {
			errors.push(`application link ${link.id} was missing`);
			continue;
		}
		for (const [field, value] of [
			["app_id", link.appId],
			["alias", link.alias],
		] as const) {
			expectField(remote, field, value, `application link ${link.id}`, errors);
		}
		const purpose =
			remote.purpose === "SharedTemplate"
				? "SHARED_TEMPLATE"
				: typeof remote.purpose === "string"
					? remote.purpose.toUpperCase()
					: remote.purpose;
		if (purpose !== link.purpose) {
			errors.push(`application link ${link.id}.purpose did not match the plan`);
		}
	}
	if (errors.length > 0) {
		const suffix = errors.length > 8 ? `; and ${errors.length - 8} more` : "";
		throw new Error(
			`Remote verification failed: ${errors.slice(0, 8).join("; ")}${suffix}.`,
		);
	}
}

function safeUrl(value: string): string {
	try {
		const url = new URL(value);
		url.username = "";
		url.password = "";
		url.search = "";
		url.hash = "";
		return url.toString();
	} catch {
		return value;
	}
}

function sanitizeEmbeddedSecrets(value: string): string {
	return value
		.replace(/pat_[A-Za-z0-9._-]+/g, "[REDACTED]")
		.replace(/https?:\/\/[^\s\"'<>\\]+/g, (match) => safeUrl(match));
}

function sanitizeRemoteValue(value: unknown, key?: string): unknown {
	if (key === "attempts") return undefined;
	if (key === "signed_url" || key === "signedUrl") return undefined;
	if (Array.isArray(value)) {
		return value.map((item) => sanitizeRemoteValue(item));
	}
	if (value && typeof value === "object") {
		const output: Record<string, unknown> = {};
		for (const [childKey, item] of Object.entries(
			value as Record<string, unknown>,
		)) {
			const sanitized = sanitizeRemoteValue(item, childKey);
			if (sanitized !== undefined) output[childKey] = sanitized;
		}
		return output;
	}
	if (
		typeof value === "string" &&
		(key?.endsWith("_url") || key?.endsWith("Url"))
	) {
		return safeUrl(value);
	}
	if (typeof value === "string") return sanitizeEmbeddedSecrets(value);
	return value;
}

async function inspectData(
	client: UniversityClient,
	courseId: string,
	language?: string,
): Promise<unknown> {
	const structure = record(
		await client.getCourseStructure(courseId, language),
		"Course structure",
	);
	const modules = array(structure.modules, "Course modules");
	const detailedModules = await Promise.all(
		modules.map(async (moduleValue) => {
			const module = record(moduleValue, "Course module");
			const moduleId = stringField(module.id, "Course module id");
			const lessons = array(module.lessons, "Lesson summaries");
			const details = await Promise.all(
				lessons.map(async (lessonValue) => {
					const lesson = record(lessonValue, "Lesson summary");
					return client.getLesson(
						courseId,
						moduleId,
						stringField(lesson.id, "Lesson id"),
					);
				}),
			);
			return { ...module, lessons: details };
		}),
	);
	const [assets, appLinks] = await Promise.all([
		client.listCourseAssets(courseId),
		client.listAppLinks(courseId),
	]);
	return sanitizeRemoteValue({
		course: structure.course,
		modules: detailedModules,
		assets,
		app_links: appLinks,
	});
}

export function planUniversityRun(
	plan: UniversityPlan,
): UniversityCommandResult {
	const startedAt = Date.now();
	const operations = operationResults(executionSteps(plan), "planned");
	return {
		schema: UNIVERSITY_RESULT_SCHEMA,
		command: "plan",
		passed: true,
		durationMs: elapsed(startedAt),
		courseId: plan.course.id,
		summary: `${operations.length} operation${operations.length === 1 ? "" : "s"} validated; no API requests made.`,
		operations,
	};
}

export async function runUniversityPlan(
	plan: UniversityPlan,
	options: UniversityRemoteOptions,
): Promise<UniversityCommandResult> {
	const startedAt = Date.now();
	const steps = executionSteps(plan);
	const operations = operationResults(steps, "planned");
	const runtime = remoteRuntime(options);
	const state: ApplyState = {};
	try {
		for (let index = 0; index < steps.length; index += 1) {
			const stepStartedAt = Date.now();
			try {
				if (steps[index].verify) {
					await verifyRemotePlan(runtime.client, plan);
					operations[index].status = "completed";
				} else {
					operations[index].status = await applyOperation(
						runtime.client,
						plan.course.id,
						steps[index].operation as UniversityOperation,
						state,
					);
				}
				operations[index].durationMs = elapsed(stepStartedAt);
			} catch (error) {
				const message = errorMessage(error);
				operations[index].status = "failed";
				operations[index].durationMs = elapsed(stepStartedAt);
				operations[index].error = message;
				for (const pending of operations.slice(index + 1)) {
					pending.status = "skipped";
				}
				return {
					schema: UNIVERSITY_RESULT_SCHEMA,
					command: "apply",
					passed: false,
					durationMs: elapsed(startedAt),
					courseId: plan.course.id,
					summary: "Apply stopped before the remaining operations were run.",
					error: message,
					operations,
				};
			}
		}
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "apply",
			passed: true,
			durationMs: elapsed(startedAt),
			courseId: plan.course.id,
			summary: plan.course.isPublished
				? "Course applied, verified, and published."
				: "Course applied and verified as a draft.",
			operations,
			data: {
				published: plan.course.isPublished,
				modules: plan.course.modules.length,
				lessons: plan.course.modules.reduce(
					(total, module) => total + module.lessons.length,
					0,
				),
				challenges: plan.course.modules.reduce(
					(total, module) =>
						total +
						module.lessons.reduce(
							(lessonTotal, lesson) => lessonTotal + lesson.challenges.length,
							0,
						),
					0,
				),
				assets: plan.course.assets.length,
			},
		};
	} finally {
		runtime.close();
	}
}

export async function inspectUniversityCourse(
	courseId: string,
	language: string | undefined,
	options: UniversityRemoteOptions,
): Promise<UniversityCommandResult> {
	const startedAt = Date.now();
	const operation: UniversityOperationResult = {
		index: 0,
		type: "inspect",
		status: "planned",
		description: `Inspect the complete course ${courseId}`,
	};
	const runtime = remoteRuntime(options);
	try {
		const data = await inspectData(runtime.client, courseId, language);
		operation.status = "completed";
		operation.durationMs = elapsed(startedAt);
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "inspect",
			passed: true,
			durationMs: elapsed(startedAt),
			courseId,
			summary:
				"Course structure, lesson bodies, challenges, references, and assets loaded.",
			operations: [operation],
			data,
		};
	} catch (error) {
		const message = errorMessage(error);
		operation.status = "failed";
		operation.durationMs = elapsed(startedAt);
		operation.error = message;
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "inspect",
			passed: false,
			durationMs: elapsed(startedAt),
			courseId,
			error: message,
			operations: [operation],
		};
	} finally {
		runtime.close();
	}
}

export async function listUniversityCourses(
	language: string | undefined,
	options: UniversityRemoteOptions,
): Promise<UniversityCommandResult> {
	const startedAt = Date.now();
	const operation: UniversityOperationResult = {
		index: 0,
		type: "list",
		status: "planned",
		description: "List all University courses, including drafts",
	};
	const runtime = remoteRuntime(options);
	try {
		const courses: unknown[] = [];
		for (let offset = 0; ; offset += 100) {
			const page = array(
				await runtime.client.listCourses({
					language,
					include_unpublished: true,
					limit: 100,
					offset,
				}),
				"Course list",
			);
			courses.push(...page);
			if (page.length < 100) break;
			if (offset >= 99_900) {
				throw new Error("Course list exceeded the 100000 item safety limit.");
			}
		}
		operation.status = "completed";
		operation.durationMs = elapsed(startedAt);
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "list",
			passed: true,
			durationMs: elapsed(startedAt),
			summary: `${courses.length} course${courses.length === 1 ? "" : "s"} loaded.`,
			operations: [operation],
			data: sanitizeRemoteValue(courses),
		};
	} catch (error) {
		const message = errorMessage(error);
		operation.status = "failed";
		operation.durationMs = elapsed(startedAt);
		operation.error = message;
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "list",
			passed: false,
			durationMs: elapsed(startedAt),
			error: message,
			operations: [operation],
		};
	} finally {
		runtime.close();
	}
}

export async function uploadSingleUniversityAsset(
	input: UploadSingleUniversityAssetOptions,
	options: UniversityRemoteOptions,
): Promise<UniversityCommandResult> {
	const startedAt = Date.now();
	const operation: UniversityOperationResult = {
		index: 0,
		type: "uploadAsset",
		status: "planned",
		description: `Upload asset @${input.name} from ${basename(input.file)}`,
	};
	const runtime = remoteRuntime(options);
	try {
		const info = await stat(input.file);
		if (!info.isFile())
			throw new Error(`Asset path is not a regular file: ${input.file}`);
		if (info.size > 2_147_483_647) {
			throw new Error(
				"Asset exceeds the University API's 2147483647 byte limit.",
			);
		}
		const extension = extensionFor(input.file);
		const mimeType = input.mimeType?.toLowerCase() ?? mimeTypeFor(input.file);
		const kind = input.kind ?? assetKindFor(mimeType);
		const filename = basename(input.file);
		if (new TextEncoder().encode(filename).byteLength > 255) {
			throw new Error("Asset filename cannot exceed 255 UTF-8 bytes.");
		}
		if (new TextEncoder().encode(mimeType).byteLength > 255) {
			throw new Error("Asset MIME type cannot exceed 255 bytes.");
		}
		if (kind !== "DOCUMENT" && !mimeType.startsWith(`${kind.toLowerCase()}/`)) {
			throw new Error(
				`Asset kind ${kind} does not match MIME type ${mimeType}.`,
			);
		}
		const asset: UniversityAssetPlan = {
			name: input.name,
			file: input.file,
			kind,
			mimeType,
			filename,
			replace: input.replace,
			size: info.size,
			extension,
		};
		const state: ApplyState = {};
		operation.status = await applyAsset(
			runtime.client,
			input.courseId,
			asset,
			state,
		);
		operation.durationMs = elapsed(startedAt);
		const remote = state.assets?.find((item) => item.name === input.name);
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "asset",
			passed: true,
			durationMs: elapsed(startedAt),
			courseId: input.courseId,
			summary:
				operation.status === "skipped"
					? `Asset @${input.name} already matched the local file.`
					: `Asset @${input.name} uploaded.`,
			operations: [operation],
			data: remote ? sanitizeRemoteValue(remote.raw) : undefined,
		};
	} catch (error) {
		const message = errorMessage(error);
		operation.status = "failed";
		operation.durationMs = elapsed(startedAt);
		operation.error = message;
		return {
			schema: UNIVERSITY_RESULT_SCHEMA,
			command: "asset",
			passed: false,
			durationMs: elapsed(startedAt),
			courseId: input.courseId,
			error: message,
			operations: [operation],
		};
	} finally {
		runtime.close();
	}
}
