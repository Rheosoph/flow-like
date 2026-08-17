import type {
	ChallengeKind,
	CourseAppPurpose,
	CourseAssetKind,
	CourseCategory,
	CourseDifficulty,
	LessonAppRefKind,
} from "./api-types";

export const UNIVERSITY_PLAN_SCHEMA = "flow-like.university-plan/v1" as const;

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
	| JsonPrimitive
	| JsonValue[]
	| { [key: string]: JsonValue };

export interface UniversityMediaPlan {
	/** Absolute path, resolved relative to the plan file when loaded from disk. */
	icon?: string;
	/** Absolute path, resolved relative to the plan file when loaded from disk. */
	banner?: string;
}

export interface UniversityAssetPlan {
	name: string;
	/** Absolute path, resolved relative to the plan file when loaded from disk. */
	file: string;
	kind: CourseAssetKind;
	mimeType: string;
	filename: string;
	replace: boolean;
	size: number;
	extension: string;
}

export interface UniversityAppLinkPlan {
	id: string;
	appId: string;
	purpose: CourseAppPurpose;
	alias: string | null;
}

export interface UniversityChallengePlan {
	id: string;
	kind: ChallengeKind;
	prompt: string;
	explanation: string | null;
	payload: JsonValue;
	points: number;
	position: number;
}

export interface UniversityAppRefPlan {
	id: string;
	kind: LessonAppRefKind;
	target: JsonValue;
	appAlias: string | null;
	appId: string | null;
	label: string | null;
}

export interface UniversityLessonPlan {
	id: string;
	title: string;
	language: string;
	/** Fully materialized content. `contentFile` is retained for provenance. */
	content: string;
	/** Absolute source path when the lesson used contentFile. */
	contentFile?: string;
	videoUrl: string | null;
	estimatedMinutes: number;
	position: number;
	isOptional: boolean;
	finalAssessment: boolean;
	challenges: UniversityChallengePlan[];
	appRefs: UniversityAppRefPlan[];
}

export interface UniversityModulePlan {
	id: string;
	title: string;
	description: string | null;
	position: number;
	lessons: UniversityLessonPlan[];
}

export interface UniversityCoursePlan {
	id: string;
	name: string;
	language: string;
	slug: string | null;
	difficulty: CourseDifficulty;
	category: CourseCategory;
	estimatedMinutes: number;
	isPublished: boolean;
	iconUrl: string | null;
	bannerUrl: string | null;
	tags: string[];
	position: number | null;
	description: string | null;
	longDescription: string | null;
	media?: UniversityMediaPlan;
	assets: UniversityAssetPlan[];
	appLinks: UniversityAppLinkPlan[];
	modules: UniversityModulePlan[];
}

export interface UniversityPlan {
	schema: typeof UNIVERSITY_PLAN_SCHEMA;
	course: UniversityCoursePlan;
}

export type UniversityOperation =
	| { type: "upsertCourse"; course: UniversityCoursePlan; publish: boolean }
	| {
			type: "uploadMedia";
			item: "icon" | "banner";
			file: string;
			language: string;
	  }
	| { type: "uploadAsset"; asset: UniversityAssetPlan }
	| { type: "upsertAppLink"; appLink: UniversityAppLinkPlan }
	| { type: "upsertModule"; module: UniversityModulePlan }
	| {
			type: "upsertLesson";
			moduleId: string;
			lesson: UniversityLessonPlan;
	  }
	| {
			type: "upsertChallenge";
			lessonId: string;
			challenge: UniversityChallengePlan;
	  }
	| {
			type: "upsertAppRef";
			lessonId: string;
			appRef: UniversityAppRefPlan;
	  };

export type UniversityOperationKind = UniversityOperation["type"];
