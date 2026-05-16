export type CourseDifficulty = "BEGINNER" | "INTERMEDIATE" | "ADVANCED" | "EXPERT";

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

export type CourseAppPurpose = "SHARED_TEMPLATE" | "REFERENCE" | "PLAYGROUND";

export type LessonAppRefKind =
	| "NAVIGATE"
	| "FOCUS_NODE"
	| "ADD_NODE"
	| "CREATE_EVENT"
	| "OPEN_OR_CLONE_APP";

export type ChallengeKind =
	| "SINGLE_CHOICE"
	| "MULTIPLE_CHOICE"
	| "BOARD_RIDDLE"
	| "EXECUTE_NODE";

export type LessonStatus = "NOT_STARTED" | "IN_PROGRESS" | "COMPLETED";

export interface CourseListItem {
	readonly id: string;
	readonly language: string;
	readonly slug: string | null;
	readonly difficulty: CourseDifficulty;
	readonly category: CourseCategory;
	readonly estimated_minutes: number;
	readonly is_published: boolean;
	readonly icon_url: string | null;
	readonly banner_url: string | null;
	readonly tags: ReadonlyArray<string>;
	readonly position: number | null;
	readonly name: string | null;
	readonly description: string | null;
}

export interface CourseDetail extends CourseListItem {
	readonly long_description: string | null;
}

export type CourseAssetKind = "IMAGE" | "VIDEO" | "AUDIO" | "DOCUMENT";

export interface CourseAsset {
	readonly id: string;
	readonly course_id: string;
	readonly name: string;
	readonly filename: string;
	readonly mime_type: string;
	readonly size: number;
	readonly kind: CourseAssetKind;
	readonly created_at: string;
	readonly updated_at: string;
}

export interface CreateCourseAssetBody {
	name: string;
	filename: string;
	mime_type: string;
	size: number;
	kind: CourseAssetKind;
	extension: string;
}

export interface CreateCourseAssetResponse {
	readonly asset: CourseAsset;
	readonly signed_url: string;
}

export interface OptimizeCourseAssetResponse {
	readonly asset: CourseAsset;
	readonly previous_size: number;
	readonly previous_mime_type: string;
}

export interface LearningPathStep {
	readonly course_id: string;
	readonly position: number;
	readonly course: CourseListItem | null;
}

export interface LearningPath {
	readonly id: string;
	readonly title: string;
	readonly description: string | null;
	readonly slug: string | null;
	readonly position: number;
	readonly is_published: boolean;
	readonly steps: ReadonlyArray<LearningPathStep>;
}

export interface CourseModule {
	readonly id: string;
	readonly courseId: string;
	readonly title: string;
	readonly description: string | null;
	readonly position: number;
}

export interface Lesson {
	readonly id: string;
	readonly module_id: string;
	readonly title: string;
	readonly position: number;
	readonly language: string;
	readonly content: string;
	readonly video_url: string | null;
	readonly estimated_minutes: number;
	readonly is_optional: boolean;
}

export interface ChallengeChoiceOption {
	readonly id: string;
	readonly label: string;
}

export interface ChoiceChallengePayload {
	readonly options: ReadonlyArray<ChallengeChoiceOption>;
	readonly correct: ReadonlyArray<string>;
}

export interface BoardRiddlePayload {
	readonly boardId?: string;
	readonly predicates: ReadonlyArray<{
		readonly op:
			| "requires_nodes"
			| "forbids_nodes"
			| "max_nodes"
			| "min_nodes";
		readonly args: ReadonlyArray<string | number>;
	}>;
}

export interface ExecuteNodeChallengePayload {
	readonly appAlias?: string;
	readonly app_alias?: string;
	readonly appId?: string;
	readonly app_id?: string;
	readonly boardId: string;
	readonly board_id?: string;
	readonly nodeId: string;
	readonly requiredPackages?: ReadonlyArray<string>;
	readonly required_packages?: ReadonlyArray<string>;
	readonly packages?: ReadonlyArray<string>;
}

export interface Challenge {
	readonly id: string;
	readonly lesson_id: string;
	readonly position: number;
	readonly kind: ChallengeKind;
	readonly prompt: string;
	readonly explanation: string | null;
	readonly payload:
		| ChoiceChallengePayload
		| BoardRiddlePayload
		| ExecuteNodeChallengePayload
		| Record<string, unknown>;
	readonly points: number;
}

export interface NavigateRefTarget {
	readonly subpath: string;
	readonly params?: Record<string, string>;
}

export interface FocusNodeRefTarget {
	readonly boardId: string;
	readonly nodeId: string;
}

export interface AddNodeRefTarget {
	readonly boardId: string;
	readonly nodeTypeId: string;
	readonly coords?: readonly [number, number];
}

export interface CreateEventRefTarget {
	readonly template: Record<string, unknown>;
}

export interface OpenOrCloneAppRefTarget {
	readonly sharedAppId: string;
	readonly alias?: string;
}

export interface LessonAppRef {
	readonly id: string;
	readonly lesson_id: string;
	readonly app_alias: string | null;
	readonly app_id: string | null;
	readonly kind: LessonAppRefKind;
	readonly target:
		| NavigateRefTarget
		| FocusNodeRefTarget
		| AddNodeRefTarget
		| CreateEventRefTarget
		| OpenOrCloneAppRefTarget;
	readonly label: string | null;
}

export interface LessonSummary {
	readonly id: string;
	readonly module_id: string;
	readonly title: string;
	readonly position: number;
	readonly estimated_minutes: number;
	readonly is_optional: boolean;
	readonly has_video: boolean;
}

export interface ModuleWithLessons {
	readonly id: string;
	readonly course_id: string;
	readonly title: string;
	readonly description: string | null;
	readonly position: number;
	readonly lessons: ReadonlyArray<LessonSummary>;
}

export interface CourseStructure {
	readonly course: CourseDetail;
	readonly modules: ReadonlyArray<ModuleWithLessons>;
}

export interface LessonAssetView {
	readonly id: string;
	readonly name: string;
	readonly mime_type: string;
	readonly kind: CourseAssetKind;
	readonly signed_url: string;
}

export interface LessonWithChildren {
	readonly lesson: Lesson;
	readonly challenges: ReadonlyArray<Challenge>;
	readonly app_refs: ReadonlyArray<LessonAppRef>;
	readonly attempts: ReadonlyArray<ChallengeAttempt>;
	readonly assets: ReadonlyArray<LessonAssetView>;
}

export interface ForkIdMap {
	readonly source_app_id: string;
	readonly app_id: string;
	readonly boards: Record<string, string>;
	readonly nodes: Record<string, string>;
	readonly pins: Record<string, string>;
	readonly events: Record<string, string>;
	readonly pages: Record<string, string>;
	readonly layers: Record<string, string>;
}

export interface UserCourseEnrollment {
	readonly id: string;
	readonly user_id: string;
	readonly course_id: string;
	readonly linked_app_ids: Record<string, string>;
	readonly id_maps: Record<string, ForkIdMap>;
	readonly started_at: string;
	readonly last_seen_at: string;
	readonly completed_at: string | null;
}

/** Resolves a source ID to a user-specific one given an enrollment + alias. */
export function translateId(
	enrollment: UserCourseEnrollment | null | undefined,
	alias: string | null | undefined,
	kind: "boards" | "nodes" | "pins" | "events" | "pages" | "layers",
	srcId: string | null | undefined,
): string | null {
	if (!srcId) return null;
	if (!enrollment || !alias) return srcId;
	const table = enrollment.id_maps?.[alias]?.[kind];
	if (!table) return srcId;
	return table[srcId] ?? srcId;
}

export interface UserLessonProgress {
	readonly id: string;
	readonly user_id: string;
	readonly lesson_id: string;
	readonly status: LessonStatus;
	readonly completed_at: string | null;
}

export interface AttemptResult {
	readonly is_correct: boolean;
	readonly points_awarded: number;
	readonly explanation: string | null;
	readonly attempt_id: string;
}

export interface ChallengeAttempt {
	readonly id: string;
	readonly challenge_id: string;
	readonly submission: unknown;
	readonly is_correct: boolean;
	readonly points_awarded: number;
	readonly attempted_at: string;
}

export interface CertificateView {
	readonly id: string;
	readonly user_id: string;
	readonly course_id: string;
	readonly issued_at: string;
	readonly hash: string;
	readonly pdf_url: string | null;
	readonly recipient_name: string | null;
	readonly course_name: string | null;
}

export interface LeaderboardEntry {
	readonly user_id: string;
	readonly display_name: string;
	readonly avatar_url: string | null;
	readonly total_points: number;
}

export interface LeaderboardOptIn {
	readonly user_id: string;
	readonly display_name: string;
	readonly is_opted_in: boolean;
	readonly total_points: number;
}

export interface CurrentWeekly {
	readonly week_iso: string;
	readonly challenge: Challenge | null;
	readonly expires_at: string | null;
}

export type LessonAction =
	| {
			kind: "NAVIGATE";
			appId: string | null;
			appAlias?: string;
			subpath: string;
			params?: Record<string, string>;
	  }
	| {
			kind: "FOCUS_NODE";
			appId: string | null;
			appAlias?: string;
			boardId: string;
			nodeId: string;
	  }
	| {
			kind: "ADD_NODE";
			appId: string | null;
			appAlias?: string;
			boardId: string;
			nodeTypeId: string;
			coords?: readonly [number, number];
	  }
	| {
			kind: "CREATE_EVENT";
			appId: string | null;
			appAlias?: string;
			template: Record<string, unknown>;
	  }
	| { kind: "OPEN_OR_CLONE_APP"; sharedAppId: string | null; alias?: string };
