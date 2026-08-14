export type CourseDifficulty =
	| "BEGINNER"
	| "INTERMEDIATE"
	| "ADVANCED"
	| "EXPERT";

export type CourseCategory =
	| "GENERAL"
	| "GETTING_STARTED"
	| "FLOWS"
	| "PAGES"
	| "EVENTS"
	| "DATA"
	| "AI"
	| "INTEGRATIONS"
	| "DEPLOYMENT"
	| "ADVANCED"
	| "EXPERT";

export type CourseAssetKind = "IMAGE" | "VIDEO" | "AUDIO" | "DOCUMENT";

export type ChallengeKind =
	| "SINGLE_CHOICE"
	| "MULTIPLE_CHOICE"
	| "BOARD_RIDDLE"
	| "EXECUTE_NODE";

export type CourseAppPurpose = "SHARED_TEMPLATE" | "REFERENCE" | "PLAYGROUND";

export type LessonAppRefKind =
	| "NAVIGATE"
	| "FOCUS_NODE"
	| "ADD_NODE"
	| "CREATE_EVENT"
	| "OPEN_OR_CLONE_APP";

export type CourseMediaItem = "icon" | "thumbnail";

export interface UniversityClientConfig {
	baseUrl: string;
	pat: string;
	/** Default cancellation signal for API requests and signed uploads. */
	signal?: AbortSignal;
}

export interface ListCoursesQuery {
	language?: string;
	category?: CourseCategory;
	difficulty?: CourseDifficulty;
	include_unpublished?: boolean;
	limit?: number;
	offset?: number;
}

export interface CourseUpsertBody {
	language: string;
	slug: string | null;
	difficulty: CourseDifficulty;
	category: CourseCategory;
	estimated_minutes: number;
	is_published: boolean;
	icon_url: string | null;
	banner_url: string | null;
	tags: string[];
	position: number | null;
	name: string;
	description: string | null;
	long_description: string | null;
}

export interface ModuleUpsertBody {
	title: string;
	description: string | null;
	position: number;
}

export interface LessonUpsertBody {
	title: string;
	language: string;
	content: string;
	video_url: string | null;
	estimated_minutes: number;
	position: number;
	is_optional: boolean;
}

export interface ChallengeUpsertBody {
	kind: ChallengeKind;
	prompt: string;
	explanation: string | null;
	payload: unknown;
	points: number;
	position: number;
}

export interface AppLinkUpsertBody {
	app_id: string;
	purpose: CourseAppPurpose;
	alias: string | null;
}

export interface AppRefUpsertBody {
	kind: LessonAppRefKind;
	target: unknown;
	app_alias: string | null;
	app_id: string | null;
	label: string | null;
}

export interface CreateCourseAssetBody {
	name: string;
	filename: string;
	mime_type: string;
	size: number;
	kind: CourseAssetKind;
	extension: string;
}

export interface UpdateCourseAssetBody {
	name: string;
}

export interface CourseMediaUploadQuery {
	language?: string;
	item: CourseMediaItem;
	extension: string;
}

export interface CourseListItem {
	id: string;
	language: string;
	slug: string | null;
	difficulty: CourseDifficulty;
	category: CourseCategory;
	estimated_minutes: number;
	is_published: boolean;
	icon_url: string | null;
	banner_url: string | null;
	tags: string[];
	position: number | null;
	name: string | null;
	description: string | null;
}

export interface CourseDetail extends CourseListItem {
	long_description: string | null;
}

export interface LessonSummary {
	id: string;
	module_id: string;
	title: string;
	position: number;
	estimated_minutes: number;
	is_optional: boolean;
	has_video: boolean;
}

export interface ModuleWithLessons {
	id: string;
	course_id: string;
	title: string;
	description: string | null;
	position: number;
	lessons: LessonSummary[];
}

export interface CourseStructure {
	course: CourseDetail;
	modules: ModuleWithLessons[];
}

export interface CourseModule {
	id: string;
	course_id: string;
	title: string;
	description: string | null;
	position: number;
	created_at: string;
	updated_at: string;
}

export interface Lesson {
	id: string;
	module_id: string;
	title: string;
	position: number;
	language: string;
	content: string;
	video_url: string | null;
	estimated_minutes: number;
	is_optional: boolean;
	created_at: string;
	updated_at: string;
}

export interface Challenge {
	id: string;
	lesson_id: string;
	position: number;
	kind: ChallengeKind;
	prompt: string;
	explanation: string | null;
	payload: unknown;
	points: number;
}

export interface LessonAppRef {
	id: string;
	lesson_id: string;
	app_alias: string | null;
	app_id: string | null;
	kind: LessonAppRefKind;
	target: unknown;
	label: string | null;
}

/** SeaORM currently serializes this enum using its Rust variant names. */
export type CourseAppPurposeResponse =
	| CourseAppPurpose
	| "SharedTemplate"
	| "Reference"
	| "Playground";

export interface CourseAppLink {
	id: string;
	course_id: string;
	app_id: string;
	purpose: CourseAppPurposeResponse;
	alias: string | null;
	created_at: string;
	updated_at: string;
}

export interface ChallengeAttempt {
	id: string;
	challenge_id: string;
	submission: unknown;
	is_correct: boolean;
	points_awarded: number;
	attempted_at: string;
}

export interface LessonAsset {
	id: string;
	name: string;
	mime_type: string;
	kind: CourseAssetKind;
	signed_url: string;
}

export interface LessonWithChildren {
	lesson: Lesson;
	challenges: Challenge[];
	app_refs: LessonAppRef[];
	attempts: ChallengeAttempt[];
	assets: LessonAsset[];
}

export interface CourseAsset {
	id: string;
	course_id: string;
	name: string;
	filename: string;
	mime_type: string;
	size: number;
	kind: CourseAssetKind;
	created_at: string;
	updated_at: string;
}

export interface CreateCourseAssetResponse {
	asset: CourseAsset;
	signed_url: string;
}

export interface OptimizeCourseAssetResponse {
	asset: CourseAsset;
	previous_size: number;
	previous_mime_type: string;
}

export interface CourseMediaUploadResponse {
	signed_url: string;
}

export type UniversityUploadBody = Blob | ArrayBuffer | Uint8Array;

export interface SignedUploadOptions {
	contentType?: string;
	headers?: HeadersInit;
	signal?: AbortSignal;
}
