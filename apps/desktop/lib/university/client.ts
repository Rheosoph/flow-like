import type {
	AppLinkUpsertBody,
	AppRefUpsertBody,
	Challenge,
	ChallengeUpsertBody,
	CourseAppLink,
	CourseAsset,
	CourseAssetKind,
	CourseDetail,
	CourseListItem,
	CourseMediaUploadQuery,
	CourseMediaUploadResponse,
	CourseModule,
	CourseStructure,
	CourseUpsertBody,
	CreateCourseAssetBody,
	CreateCourseAssetResponse,
	Lesson,
	LessonAppRef,
	LessonUpsertBody,
	LessonWithChildren,
	ListCoursesQuery,
	ModuleUpsertBody,
	OptimizeCourseAssetResponse,
	SignedUploadOptions,
	UniversityClientConfig,
	UniversityUploadBody,
} from "./api-types";

type QueryValue = string | number | boolean | undefined;

interface RequestOptions {
	query?: Record<string, QueryValue>;
	body?: unknown;
	response?: "object" | "array" | "void";
	signal?: AbortSignal | null;
}

interface UniversityApiErrorOptions {
	status: number;
	statusText: string;
	method: string;
	url: string;
	code?: string;
	errorId?: string;
	body?: unknown;
}

export class UniversityApiError extends Error {
	readonly status: number;
	readonly statusText: string;
	readonly method: string;
	readonly url: string;
	readonly code?: string;
	readonly errorId?: string;
	readonly body?: unknown;

	constructor(message: string, options: UniversityApiErrorOptions) {
		super(message);
		this.name = "UniversityApiError";
		this.status = options.status;
		this.statusText = options.statusText;
		this.method = options.method;
		this.url = options.url;
		this.code = options.code;
		this.errorId = options.errorId;
		Object.defineProperty(this, "body", {
			configurable: false,
			enumerable: false,
			value: options.body,
		});
	}
}

const SENSITIVE_QUERY_KEY =
	/(?:token|key|secret|password|passwd|auth|code|signature|credential|sig|sas)/i;

function safeErrorUrl(value: string): string {
	try {
		const url = new URL(value);
		url.username = "";
		url.password = "";
		for (const key of [...url.searchParams.keys()]) {
			if (SENSITIVE_QUERY_KEY.test(key))
				url.searchParams.set(key, "[REDACTED]");
		}
		url.hash = "";
		return url.toString();
	} catch {
		return value;
	}
}

function safeErrorMessage(value: string): string {
	return value
		.replace(/pat_[A-Za-z0-9._-]+/g, "[REDACTED]")
		.replace(
			/([?&][^=&\s]*(?:token|key|secret|password|passwd|auth|code|signature|credential|sig|sas)[^=&\s]*=)[^&\s]+/gi,
			"$1[REDACTED]",
		);
}

function objectValue(value: unknown): Record<string, unknown> | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function stringValue(value: unknown): string | undefined {
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

async function responsePayload(response: Response): Promise<{
	text: string;
	value: unknown;
}> {
	const text = await response.text();
	if (!text) return { text, value: undefined };
	try {
		return { text, value: JSON.parse(text) };
	} catch {
		return { text, value: text };
	}
}

async function errorFromResponse(
	response: Response,
	method: string,
	url: string,
): Promise<UniversityApiError> {
	let text = "";
	let value: unknown;
	try {
		const payload = await responsePayload(response);
		text = payload.text;
		value = payload.value;
	} catch {
		value = undefined;
	}

	const top = objectValue(value);
	const nested = objectValue(top?.error);
	const code = stringValue(nested?.code) ?? stringValue(top?.code);
	const errorId =
		stringValue(nested?.id) ??
		stringValue(top?.id) ??
		response.headers.get("x-error-id") ??
		undefined;
	const fallbackMessage =
		text.trim() ||
		`${method} request failed with ${response.status} ${response.statusText}`.trim();
	const message = safeErrorMessage(
		stringValue(nested?.message) ??
			stringValue(top?.message) ??
			fallbackMessage,
	);

	return new UniversityApiError(message, {
		status: response.status,
		statusText: response.statusText,
		method,
		url: safeErrorUrl(url),
		code,
		errorId,
		body: value,
	});
}

function encodePathSegment(value: string): string {
	return encodeURIComponent(value).replace(
		/[!'()*]/g,
		(character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
	);
}

function appendQuery(url: URL, query?: Record<string, QueryValue>): void {
	if (!query) return;
	for (const [key, value] of Object.entries(query)) {
		if (value !== undefined) url.searchParams.set(key, String(value));
	}
}

export function normalizeUniversityBaseUrl(baseUrl: string): string {
	const input = baseUrl.trim();
	if (!input) throw new Error("University API base URL is required.");

	let url: URL;
	try {
		url = new URL(input);
	} catch {
		throw new Error(`Invalid University API base URL: ${baseUrl}`);
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") {
		throw new Error("University API base URL must use http or https.");
	}
	if (url.protocol === "http:" && !isLoopbackHostname(url.hostname)) {
		throw new Error(
			"University API base URL must use https unless it targets loopback development.",
		);
	}
	if (url.username || url.password) {
		throw new Error("University API base URL must not contain credentials.");
	}

	url.search = "";
	url.hash = "";
	let path = url.pathname.replace(/\/+$/, "");
	if (!path.toLowerCase().endsWith("/api/v1")) path = `${path}/api/v1`;
	url.pathname = path || "/api/v1";
	return url.toString().replace(/\/$/, "");
}

function isLoopbackHostname(hostname: string): boolean {
	const value = hostname.toLowerCase().replace(/^\[|\]$/g, "");
	if (
		value === "localhost" ||
		value.endsWith(".localhost") ||
		value === "::1"
	) {
		return true;
	}
	const match = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
	if (!match || match.slice(1).some((part) => Number(part) > 255)) return false;
	return Number(match[1]) === 127;
}

function isAzureBlobStorageUrl(value: string): boolean {
	try {
		return new URL(value).hostname
			.toLowerCase()
			.endsWith(".blob.core.windows.net");
	} catch {
		return false;
	}
}

function uploadBody(body: UniversityUploadBody): BodyInit {
	if (body instanceof Blob || body instanceof ArrayBuffer) return body;
	return new Uint8Array(body);
}

export async function uploadToSignedUrl(
	signedUrl: string,
	body: UniversityUploadBody,
	options: SignedUploadOptions = {},
): Promise<void> {
	let url: URL;
	try {
		url = new URL(signedUrl);
	} catch {
		throw new Error("Invalid signed upload URL.");
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") {
		throw new Error("Signed upload URL must use http or https.");
	}
	if (url.protocol === "http:" && !isLoopbackHostname(url.hostname)) {
		throw new Error(
			"Signed upload URL must use https unless it targets loopback development.",
		);
	}

	const headers = new Headers(options.headers);
	for (const name of headers.keys()) {
		if (
			/(?:authorization|cookie|token|credential|secret|api[-_]?key)/i.test(name)
		) {
			throw new Error(
				`Sensitive header ${name} is not allowed on signed uploads.`,
			);
		}
	}
	if (options.contentType && !headers.has("Content-Type")) {
		headers.set("Content-Type", options.contentType);
	}
	if (isAzureBlobStorageUrl(url.toString()) && !headers.has("x-ms-blob-type")) {
		headers.set("x-ms-blob-type", "BlockBlob");
	}

	const response = await fetch(url, {
		method: "PUT",
		headers,
		body: uploadBody(body),
		signal: options.signal,
	});
	if (!response.ok) {
		throw await errorFromResponse(response, "PUT", url.toString());
	}
}

export class UniversityClient {
	readonly baseUrl: string;
	readonly #pat: string;
	readonly #signal?: AbortSignal;

	constructor(config: UniversityClientConfig) {
		this.baseUrl = normalizeUniversityBaseUrl(config.baseUrl);
		const pat = config.pat;
		if (!pat.startsWith("pat_")) {
			throw new Error(
				"University API authentication requires a Flow-Like PAT.",
			);
		}
		if (/\s/.test(pat)) {
			throw new Error("University API PAT must not contain whitespace.");
		}
		this.#pat = pat;
		this.#signal = config.signal;
	}

	private async request<T>(
		method: string,
		path: string,
		options: RequestOptions = {},
	): Promise<T> {
		const url = new URL(`${this.baseUrl}${path}`);
		appendQuery(url, options.query);
		const headers = new Headers({
			Accept: "application/json",
			Authorization: this.#pat,
		});
		let body: string | undefined;
		if (options.body !== undefined) {
			headers.set("Content-Type", "application/json");
			body = JSON.stringify(options.body);
		}

		const response = await fetch(url, {
			method,
			headers,
			body,
			signal: options.signal === undefined ? this.#signal : options.signal,
		});
		if (!response.ok)
			throw await errorFromResponse(response, method, url.toString());

		if (options.response === "void") return undefined as T;
		const payload = await responsePayload(response);
		const responseShape = options.response ?? "object";
		if (responseShape === "array" && !Array.isArray(payload.value)) {
			throw new Error(
				`${method} ${safeErrorUrl(url.toString())} returned invalid JSON; expected an array.`,
			);
		}
		if (
			responseShape === "object" &&
			(!payload.value ||
				typeof payload.value !== "object" ||
				Array.isArray(payload.value))
		) {
			throw new Error(
				`${method} ${safeErrorUrl(url.toString())} returned invalid JSON; expected an object.`,
			);
		}
		return payload.value as T;
	}

	listCourses(query: ListCoursesQuery = {}): Promise<CourseListItem[]> {
		return this.request("GET", "/courses", {
			query: { ...query },
			response: "array",
		});
	}

	getCourse(courseId: string, language?: string): Promise<CourseDetail> {
		return this.request("GET", `/courses/${encodePathSegment(courseId)}`, {
			query: { language },
		});
	}

	getCourseStructure(
		courseId: string,
		language?: string,
	): Promise<CourseStructure> {
		return this.request(
			"GET",
			`/courses/${encodePathSegment(courseId)}/structure`,
			{ query: { language } },
		);
	}

	upsertCourse(
		courseId: string,
		body: CourseUpsertBody,
	): Promise<CourseDetail> {
		return this.request("PUT", `/courses/${encodePathSegment(courseId)}`, {
			body,
		});
	}

	deleteCourse(courseId: string): Promise<void> {
		return this.request("DELETE", `/courses/${encodePathSegment(courseId)}`, {
			response: "void",
		});
	}

	upsertModule(
		courseId: string,
		moduleId: string,
		body: ModuleUpsertBody,
	): Promise<CourseModule> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/modules/${encodePathSegment(moduleId)}`,
			{ body },
		);
	}

	deleteModule(courseId: string, moduleId: string): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/modules/${encodePathSegment(moduleId)}`,
			{ response: "void" },
		);
	}

	getLesson(
		courseId: string,
		moduleId: string,
		lessonId: string,
	): Promise<LessonWithChildren> {
		return this.request(
			"GET",
			`/courses/${encodePathSegment(courseId)}/modules/${encodePathSegment(moduleId)}/lessons/${encodePathSegment(lessonId)}`,
		);
	}

	upsertLesson(
		courseId: string,
		moduleId: string,
		lessonId: string,
		body: LessonUpsertBody,
	): Promise<Lesson> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/modules/${encodePathSegment(moduleId)}/lessons/${encodePathSegment(lessonId)}`,
			{ body },
		);
	}

	deleteLesson(
		courseId: string,
		moduleId: string,
		lessonId: string,
	): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/modules/${encodePathSegment(moduleId)}/lessons/${encodePathSegment(lessonId)}`,
			{ response: "void" },
		);
	}

	upsertChallenge(
		courseId: string,
		lessonId: string,
		challengeId: string,
		body: ChallengeUpsertBody,
	): Promise<Challenge> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/lessons/${encodePathSegment(lessonId)}/challenges/${encodePathSegment(challengeId)}`,
			{ body },
		);
	}

	deleteChallenge(
		courseId: string,
		lessonId: string,
		challengeId: string,
	): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/lessons/${encodePathSegment(lessonId)}/challenges/${encodePathSegment(challengeId)}`,
			{ response: "void" },
		);
	}

	listAppLinks(courseId: string): Promise<CourseAppLink[]> {
		return this.request(
			"GET",
			`/courses/${encodePathSegment(courseId)}/app-links`,
			{ response: "array" },
		);
	}

	upsertAppLink(
		courseId: string,
		linkId: string,
		body: AppLinkUpsertBody,
	): Promise<CourseAppLink> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/app-links/${encodePathSegment(linkId)}`,
			{ body },
		);
	}

	deleteAppLink(courseId: string, linkId: string): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/app-links/${encodePathSegment(linkId)}`,
			{ response: "void" },
		);
	}

	upsertAppRef(
		courseId: string,
		lessonId: string,
		refId: string,
		body: AppRefUpsertBody,
	): Promise<LessonAppRef> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/lessons/${encodePathSegment(lessonId)}/refs/${encodePathSegment(refId)}`,
			{ body },
		);
	}

	deleteAppRef(
		courseId: string,
		lessonId: string,
		refId: string,
	): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/lessons/${encodePathSegment(lessonId)}/refs/${encodePathSegment(refId)}`,
			{ response: "void" },
		);
	}

	listCourseAssets(
		courseId: string,
		query: { kind?: CourseAssetKind } = {},
	): Promise<CourseAsset[]> {
		return this.request(
			"GET",
			`/courses/${encodePathSegment(courseId)}/assets`,
			{ query, response: "array" },
		);
	}

	createCourseAsset(
		courseId: string,
		body: CreateCourseAssetBody,
	): Promise<CreateCourseAssetResponse> {
		return this.request(
			"POST",
			`/courses/${encodePathSegment(courseId)}/assets`,
			{ body },
		);
	}

	renameCourseAsset(
		courseId: string,
		assetId: string,
		name: string,
	): Promise<CourseAsset> {
		return this.request(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/assets/${encodePathSegment(assetId)}`,
			{ body: { name } },
		);
	}

	deleteCourseAsset(courseId: string, assetId: string): Promise<void> {
		return this.request(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/assets/${encodePathSegment(assetId)}`,
			{ response: "void" },
		);
	}

	private deleteCourseAssetForCleanup(
		courseId: string,
		assetId: string,
	): Promise<void> {
		const controller = new AbortController();
		const timeout = setTimeout(() => controller.abort(), 5_000);
		return this.request<void>(
			"DELETE",
			`/courses/${encodePathSegment(courseId)}/assets/${encodePathSegment(assetId)}`,
			{ response: "void", signal: controller.signal },
		).finally(() => clearTimeout(timeout));
	}

	optimizeCourseAsset(
		courseId: string,
		assetId: string,
	): Promise<OptimizeCourseAssetResponse> {
		return this.request(
			"POST",
			`/courses/${encodePathSegment(courseId)}/assets/${encodePathSegment(assetId)}/optimize`,
		);
	}

	async getCourseMediaUploadUrl(
		courseId: string,
		query: CourseMediaUploadQuery,
	): Promise<string> {
		const result = await this.request<CourseMediaUploadResponse>(
			"PUT",
			`/courses/${encodePathSegment(courseId)}/meta/media`,
			{ query: { ...query } },
		);
		if (typeof result.signed_url !== "string" || !result.signed_url) {
			throw new Error(
				"University API returned an invalid course media upload URL response.",
			);
		}
		return result.signed_url;
	}

	async uploadCourseMedia(
		courseId: string,
		query: CourseMediaUploadQuery,
		body: UniversityUploadBody,
		options: SignedUploadOptions = {},
	): Promise<void> {
		const signedUrl = await this.getCourseMediaUploadUrl(courseId, query);
		const bodyContentType =
			body instanceof Blob && body.type
				? body.type
				: "application/octet-stream";
		await uploadToSignedUrl(signedUrl, body, {
			...options,
			contentType: options.contentType ?? bodyContentType,
			signal: options.signal ?? this.#signal,
		});
	}

	async uploadCourseAsset(
		courseId: string,
		metadata: CreateCourseAssetBody,
		body: UniversityUploadBody,
		options: SignedUploadOptions = {},
	): Promise<CourseAsset> {
		const created = await this.createCourseAsset(courseId, metadata);
		if (
			!created.asset ||
			typeof created.asset.id !== "string" ||
			!created.asset.id ||
			typeof created.signed_url !== "string" ||
			!created.signed_url
		) {
			throw new Error(
				"University API returned an invalid asset upload response.",
			);
		}
		try {
			await uploadToSignedUrl(created.signed_url, body, {
				...options,
				contentType: options.contentType ?? metadata.mime_type,
				signal: options.signal ?? this.#signal,
			});
		} catch (uploadError) {
			try {
				await this.deleteCourseAssetForCleanup(courseId, created.asset.id);
			} catch (cleanupError) {
				if (uploadError instanceof Error) {
					uploadError.message = `${uploadError.message} Asset record cleanup also failed; retry with --replace or remove the orphaned asset manually.`;
					Object.defineProperty(uploadError, "cleanupError", {
						configurable: true,
						enumerable: false,
						value: cleanupError,
					});
				}
			}
			throw uploadError;
		}
		return created.asset;
	}
}

export function createUniversityClient(
	config: UniversityClientConfig,
): UniversityClient {
	return new UniversityClient(config);
}
