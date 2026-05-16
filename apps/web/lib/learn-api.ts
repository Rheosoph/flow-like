import { type IProfile, isAzureBlobStorageUrl } from "@tm9657/flow-like-ui";
import type {
	AttemptResult,
	CertificateView,
	Challenge,
	CourseAsset,
	CourseAssetKind,
	CourseDetail,
	CourseListItem,
	CourseModule,
	CourseStructure,
	CreateCourseAssetBody,
	CreateCourseAssetResponse,
	CurrentWeekly,
	LeaderboardEntry,
	LeaderboardOptIn,
	LearningPath,
	Lesson,
	LessonAppRef,
	LessonWithChildren,
	OptimizeCourseAssetResponse,
	UserCourseEnrollment,
	UserLessonProgress,
} from "@tm9657/flow-like-ui/lib/learn/types";
import type { AuthContextProps } from "react-oidc-context";
import { fetcher } from "./api";

export interface OpenSharedAppResponse {
	readonly course_id: string;
	readonly alias: string;
	readonly app_id: string;
	readonly source_app_id: string;
	readonly linked_now: boolean;
	readonly forked_now: boolean;
}

export type CourseMediaItem = "icon" | "thumbnail";

function qs(params: Record<string, string | number | boolean | undefined>) {
	const entries = Object.entries(params).filter(
		([, v]) => v !== undefined && v !== null && v !== "",
	);
	if (!entries.length) return "";
	const usp = new URLSearchParams();
	for (const [k, v] of entries) usp.append(k, String(v));
	return `?${usp.toString()}`;
}

export const learnApi = {
	async listCourses(
		profile: IProfile,
		auth: AuthContextProps,
		opts: {
			language?: string;
			category?: string;
			difficulty?: string;
			includeUnpublished?: boolean;
			limit?: number;
			offset?: number;
		} = {},
	): Promise<CourseListItem[]> {
		return fetcher<CourseListItem[]>(
			profile,
			`/courses${qs({
				language: opts.language,
				category: opts.category,
				difficulty: opts.difficulty,
				include_unpublished: opts.includeUnpublished,
				limit: opts.limit,
				offset: opts.offset,
			})}`,
			undefined,
			auth,
		);
	},

	async getCourse(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		language?: string,
	): Promise<CourseDetail> {
		return fetcher<CourseDetail>(
			profile,
			`/courses/${courseId}${qs({ language })}`,
			undefined,
			auth,
		);
	},

	async getCourseStructure(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		language?: string,
	): Promise<CourseStructure> {
		return fetcher<CourseStructure>(
			profile,
			`/courses/${courseId}/structure${qs({ language })}`,
			undefined,
			auth,
		);
	},

	async upsertCourse(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		body: unknown,
	): Promise<CourseDetail> {
		return fetcher<CourseDetail>(
			profile,
			`/courses/${courseId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async uploadCourseMedia(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		item: CourseMediaItem,
		file: File,
		language = "en",
	): Promise<void> {
		const signedUrl = await learnApi.getCourseMediaUploadUrl(
			profile,
			auth,
			courseId,
			item,
			file,
			language,
		);
		const headers: HeadersInit = {
			"Content-Type": file.type || "application/octet-stream",
		};
		if (isAzureBlobStorageUrl(signedUrl)) {
			headers["x-ms-blob-type"] = "BlockBlob";
		}

		const response = await fetch(signedUrl, {
			method: "PUT",
			body: file,
			headers,
		});
		if (!response.ok) {
			throw new Error(`Failed to upload course media: ${response.statusText}`);
		}
	},

	async getCourseMediaUploadUrl(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		item: CourseMediaItem,
		file: File,
		language = "en",
	): Promise<string> {
		const extension = file.name.split(".").pop() ?? "png";
		const { signed_url } = await fetcher<{ signed_url: string }>(
			profile,
			`/courses/${courseId}/meta/media${qs({
				language,
				item,
				extension,
			})}`,
			{ method: "PUT" },
			auth,
		);
		return signed_url;
	},

	async deleteCourse(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
	): Promise<void> {
		await fetcher<unknown>(
			profile,
			`/courses/${courseId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async listCourseAssets(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		opts: { kind?: CourseAssetKind } = {},
	): Promise<CourseAsset[]> {
		return fetcher<CourseAsset[]>(
			profile,
			`/courses/${courseId}/assets${qs({ kind: opts.kind })}`,
			undefined,
			auth,
		);
	},

	async createCourseAsset(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		body: CreateCourseAssetBody,
	): Promise<CreateCourseAssetResponse> {
		return fetcher<CreateCourseAssetResponse>(
			profile,
			`/courses/${courseId}/assets`,
			{ method: "POST", body: JSON.stringify(body) },
			auth,
		);
	},

	async renameCourseAsset(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		assetId: string,
		name: string,
	): Promise<CourseAsset> {
		return fetcher<CourseAsset>(
			profile,
			`/courses/${courseId}/assets/${assetId}`,
			{ method: "PUT", body: JSON.stringify({ name }) },
			auth,
		);
	},

	async deleteCourseAsset(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		assetId: string,
	): Promise<void> {
		await fetcher<unknown>(
			profile,
			`/courses/${courseId}/assets/${assetId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async optimizeCourseAsset(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		assetId: string,
	): Promise<OptimizeCourseAssetResponse> {
		return fetcher<OptimizeCourseAssetResponse>(
			profile,
			`/courses/${courseId}/assets/${assetId}/optimize`,
			{ method: "POST" },
			auth,
		);
	},

	async uploadCourseAsset(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		body: CreateCourseAssetBody,
		file: File,
	): Promise<CourseAsset> {
		const created = await learnApi.createCourseAsset(
			profile,
			auth,
			courseId,
			body,
		);
		const headers: HeadersInit = {
			"Content-Type": file.type || body.mime_type || "application/octet-stream",
		};
		if (isAzureBlobStorageUrl(created.signed_url)) {
			headers["x-ms-blob-type"] = "BlockBlob";
		}
		const response = await fetch(created.signed_url, {
			method: "PUT",
			body: file,
			headers,
		});
		if (!response.ok) {
			throw new Error(`Failed to upload asset: ${response.statusText}`);
		}
		return created.asset;
	},

	async upsertModule(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		moduleId: string,
		body: unknown,
	): Promise<CourseModule> {
		return fetcher<CourseModule>(
			profile,
			`/courses/${courseId}/modules/${moduleId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async getLesson(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		moduleId: string,
		lessonId: string,
	): Promise<LessonWithChildren> {
		return fetcher<LessonWithChildren>(
			profile,
			`/courses/${courseId}/modules/${moduleId}/lessons/${lessonId}`,
			undefined,
			auth,
		);
	},

	async upsertLesson(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		moduleId: string,
		lessonId: string,
		body: unknown,
	): Promise<Lesson> {
		return fetcher<Lesson>(
			profile,
			`/courses/${courseId}/modules/${moduleId}/lessons/${lessonId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async upsertChallenge(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		lessonId: string,
		challengeId: string,
		body: unknown,
	): Promise<Challenge> {
		return fetcher<Challenge>(
			profile,
			`/courses/${courseId}/lessons/${lessonId}/challenges/${challengeId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async listAppLinks(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
	): Promise<
		Array<{
			id: string;
			course_id: string;
			app_id: string;
			purpose: string;
			alias: string | null;
		}>
	> {
		return fetcher(profile, `/courses/${courseId}/app-links`, undefined, auth);
	},

	async upsertAppLink(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		linkId: string,
		body: { app_id: string; alias?: string | null; purpose?: string },
	): Promise<{
		id: string;
		course_id: string;
		app_id: string;
		purpose: string;
		alias: string | null;
	}> {
		return fetcher(
			profile,
			`/courses/${courseId}/app-links/${linkId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async deleteAppLink(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		linkId: string,
	): Promise<void> {
		await fetcher(
			profile,
			`/courses/${courseId}/app-links/${linkId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async deleteModule(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		moduleId: string,
	): Promise<void> {
		await fetcher(
			profile,
			`/courses/${courseId}/modules/${moduleId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async deleteLesson(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		moduleId: string,
		lessonId: string,
	): Promise<void> {
		await fetcher(
			profile,
			`/courses/${courseId}/modules/${moduleId}/lessons/${lessonId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async deleteChallenge(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		lessonId: string,
		challengeId: string,
	): Promise<void> {
		await fetcher(
			profile,
			`/courses/${courseId}/lessons/${lessonId}/challenges/${challengeId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async deleteAppRef(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		lessonId: string,
		refId: string,
	): Promise<void> {
		await fetcher(
			profile,
			`/courses/${courseId}/lessons/${lessonId}/refs/${refId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async upsertAppRef(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		lessonId: string,
		refId: string,
		body: unknown,
	): Promise<LessonAppRef> {
		return fetcher<LessonAppRef>(
			profile,
			`/courses/${courseId}/lessons/${lessonId}/refs/${refId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async enroll(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
	): Promise<UserCourseEnrollment> {
		return fetcher<UserCourseEnrollment>(
			profile,
			`/courses/${courseId}/enroll`,
			{ method: "POST" },
			auth,
		);
	},

	async myEnrollments(
		profile: IProfile,
		auth: AuthContextProps,
	): Promise<UserCourseEnrollment[]> {
		return fetcher<UserCourseEnrollment[]>(
			profile,
			"/courses/enrollments/me",
			undefined,
			auth,
		);
	},

	async markLessonComplete(
		profile: IProfile,
		auth: AuthContextProps,
		lessonId: string,
	): Promise<UserLessonProgress> {
		return fetcher<UserLessonProgress>(
			profile,
			`/courses/lessons/${lessonId}/complete`,
			{ method: "POST" },
			auth,
		);
	},

	async myCourseProgress(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
	): Promise<UserLessonProgress[]> {
		return fetcher<UserLessonProgress[]>(
			profile,
			`/courses/${courseId}/progress/me`,
			undefined,
			auth,
		);
	},

	async submitAttempt(
		profile: IProfile,
		auth: AuthContextProps,
		challengeId: string,
		submission: unknown,
	): Promise<AttemptResult> {
		return fetcher<AttemptResult>(
			profile,
			`/courses/challenges/${challengeId}/attempt`,
			{ method: "POST", body: JSON.stringify({ submission }) },
			auth,
		);
	},

	async issueCertificate(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
	): Promise<CertificateView> {
		return fetcher<CertificateView>(
			profile,
			`/courses/${courseId}/certificate`,
			{ method: "POST" },
			auth,
		);
	},

	async myCertificates(
		profile: IProfile,
		auth: AuthContextProps,
	): Promise<CertificateView[]> {
		return fetcher<CertificateView[]>(
			profile,
			"/courses/certificates/me",
			undefined,
			auth,
		);
	},

	async verifyCertificate(
		profile: IProfile,
		auth: AuthContextProps,
		certId: string,
	): Promise<CertificateView> {
		return fetcher<CertificateView>(
			profile,
			`/courses/certificates/verify/${certId}`,
			undefined,
			auth,
		);
	},

	async listLearningPaths(
		profile: IProfile,
		auth: AuthContextProps,
		opts: { language?: string; includeUnpublished?: boolean } = {},
	): Promise<LearningPath[]> {
		return fetcher<LearningPath[]>(
			profile,
			`/courses/paths${qs({
				language: opts.language,
				include_unpublished: opts.includeUnpublished,
			})}`,
			undefined,
			auth,
		);
	},

	async getLearningPath(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		opts: { language?: string } = {},
	): Promise<LearningPath> {
		return fetcher<LearningPath>(
			profile,
			`/courses/paths/${pathId}${qs({ language: opts.language })}`,
			undefined,
			auth,
		);
	},

	async upsertLearningPath(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		body: {
			title: string;
			slug?: string | null;
			description?: string | null;
			position?: number;
			is_published?: boolean;
		},
	): Promise<LearningPath> {
		return fetcher<LearningPath>(
			profile,
			`/courses/paths/${pathId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async deleteLearningPath(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
	): Promise<void> {
		await fetcher<unknown>(
			profile,
			`/courses/paths/${pathId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async upsertLearningPathStep(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		courseId: string,
		body: { position: number },
	): Promise<void> {
		await fetcher<unknown>(
			profile,
			`/courses/paths/${pathId}/courses/${courseId}`,
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async deleteLearningPathStep(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		courseId: string,
	): Promise<void> {
		await fetcher<unknown>(
			profile,
			`/courses/paths/${pathId}/courses/${courseId}`,
			{ method: "DELETE" },
			auth,
		);
	},

	async getLeaderboard(
		profile: IProfile,
		auth: AuthContextProps,
		opts: { limit?: number; offset?: number } = {},
	): Promise<LeaderboardEntry[]> {
		return fetcher<LeaderboardEntry[]>(
			profile,
			`/courses/leaderboard${qs({ limit: opts.limit, offset: opts.offset })}`,
			undefined,
			auth,
		);
	},

	async getMyOptIn(
		profile: IProfile,
		auth: AuthContextProps,
	): Promise<LeaderboardOptIn | null> {
		return fetcher<LeaderboardOptIn | null>(
			profile,
			"/courses/leaderboard/me",
			undefined,
			auth,
		);
	},

	async updateMyOptIn(
		profile: IProfile,
		auth: AuthContextProps,
		body: { display_name: string; is_opted_in: boolean },
	): Promise<LeaderboardOptIn> {
		return fetcher<LeaderboardOptIn>(
			profile,
			"/courses/leaderboard/me",
			{ method: "PUT", body: JSON.stringify(body) },
			auth,
		);
	},

	async getCurrentWeekly(
		profile: IProfile,
		auth: AuthContextProps,
	): Promise<CurrentWeekly> {
		return fetcher<CurrentWeekly>(profile, "/courses/weekly", undefined, auth);
	},

	async rotateWeekly(
		profile: IProfile,
		auth: AuthContextProps,
	): Promise<CurrentWeekly> {
		return fetcher<CurrentWeekly>(
			profile,
			"/courses/weekly/rotate",
			{ method: "POST" },
			auth,
		);
	},

	async openSharedApp(
		profile: IProfile,
		auth: AuthContextProps,
		courseId: string,
		alias: string,
		opts: { refork?: boolean; language?: string } = {},
	): Promise<OpenSharedAppResponse> {
		return fetcher<OpenSharedAppResponse>(
			profile,
			`/courses/${courseId}/links/${alias}/open${qs({
				refork: opts.refork,
				language: opts.language,
			})}`,
			{ method: "POST" },
			auth,
		);
	},
};
