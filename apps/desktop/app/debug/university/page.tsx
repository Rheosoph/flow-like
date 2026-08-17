import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import type {
	CourseCategory,
	CourseDetail,
	CourseDifficulty,
	CourseListItem,
	Lesson,
	LessonAssetView,
	ModuleWithLessons,
} from "@flow-like/flow-like-ui/lib/learn/types";
import { UniversityVisualPreview } from "./university-visual-preview";

const PLAN_PATHS = [
	"lib/university/courses/foundations/welcome-to-flow-like/course.plan.json",
	"lib/university/courses/foundations/thinking-in-flows/course.plan.json",
	"lib/university/courses/foundations/apps-for-builders/course.plan.json",
	"lib/university/courses/specialist/data-in-flow-like/course.plan.json",
	"lib/university/courses/specialist/events/course.plan.json",
	"lib/university/courses/advanced/app-governance/course.plan.json",
	"lib/university/courses/advanced/agentic-in-flow-like/course.plan.json",
	"lib/university/courses/specialist/custom-nodes/course.plan.json",
] as const;

interface PreviewAssetPlan {
	name: string;
	file: string;
	mimeType?: string;
}

interface PreviewLessonPlan {
	id: string;
	title: string;
	contentFile: string;
	estimatedMinutes: number;
	position: number;
	isOptional?: boolean;
}

interface PreviewPlan {
	course: {
		id: string;
		name: string;
		language: string;
		slug: string | null;
		difficulty: CourseDifficulty;
		category: CourseCategory;
		estimatedMinutes: number;
		isPublished: boolean;
		tags: string[];
		position: number | null;
		description: string | null;
		longDescription?: string | null;
		media: { icon: string; banner: string };
		assets: PreviewAssetPlan[];
		modules: Array<{
			id: string;
			title: string;
			description?: string | null;
			position: number;
			lessons: PreviewLessonPlan[];
		}>;
	};
}

interface LoadedPreviewPlan {
	plan: PreviewPlan;
	directory: string;
}

async function readPlan(path: string): Promise<LoadedPreviewPlan> {
	const absolutePath = resolve(process.cwd(), path);
	const source = await readFile(absolutePath, "utf8");
	return {
		plan: JSON.parse(source) as PreviewPlan,
		directory: dirname(absolutePath),
	};
}

async function imageDataUrl(path: string, mimeType = "image/webp") {
	const bytes = await readFile(path);
	return `data:${mimeType};base64,${bytes.toString("base64")}`;
}

async function courseListItem({
	plan,
	directory,
}: LoadedPreviewPlan): Promise<CourseListItem> {
	const course = plan.course;
	const [iconUrl, bannerUrl] = await Promise.all([
		imageDataUrl(resolve(directory, course.media.icon)),
		imageDataUrl(resolve(directory, course.media.banner)),
	]);
	return {
		id: course.id,
		language: course.language,
		slug: course.slug,
		difficulty: course.difficulty,
		category: course.category,
		estimated_minutes: course.estimatedMinutes,
		is_published: course.isPublished,
		icon_url: iconUrl,
		banner_url: bannerUrl,
		tags: course.tags,
		position: course.position,
		name: course.name,
		description: course.description,
	};
}

async function welcomeLesson(
	loaded: LoadedPreviewPlan,
	brokenAsset: boolean,
): Promise<{ lesson: Lesson; assets: LessonAssetView[] }> {
	const lessonPlan = loaded.plan.course.modules[0]?.lessons[0];
	if (!lessonPlan) throw new Error("Welcome course preview lesson is missing.");
	const content = await readFile(
		resolve(loaded.directory, lessonPlan.contentFile),
		"utf8",
	);
	const assets = await Promise.all(
		loaded.plan.course.assets.map(async (asset) => ({
			id: `preview-${asset.name}`,
			name: asset.name,
			mime_type: asset.mimeType ?? "image/webp",
			kind: "IMAGE" as const,
			signed_url:
				brokenAsset && asset.name === "AppAnatomy"
					? "/debug/university/missing-image.webp"
					: await imageDataUrl(
							resolve(loaded.directory, asset.file),
							asset.mimeType,
						),
		})),
	);
	return {
		lesson: {
			id: lessonPlan.id,
			module_id: loaded.plan.course.modules[0]?.id ?? "preview-module",
			title: lessonPlan.title,
			position: lessonPlan.position,
			language: loaded.plan.course.language,
			content,
			video_url: null,
			estimated_minutes: lessonPlan.estimatedMinutes,
			is_optional: lessonPlan.isOptional ?? false,
		},
		assets,
	};
}

/** The course page preview uses a multi-module course so the spine has something to draw. */
const COURSE_PREVIEW_INDEX = 1;

async function courseDetail(loaded: LoadedPreviewPlan): Promise<CourseDetail> {
	const base = await courseListItem(loaded);
	return {
		...base,
		long_description: loaded.plan.course.longDescription ?? null,
	};
}

function courseModules(loaded: LoadedPreviewPlan): ModuleWithLessons[] {
	return loaded.plan.course.modules.map((module) => ({
		id: module.id,
		course_id: loaded.plan.course.id,
		title: module.title,
		description: module.description ?? null,
		position: module.position,
		lessons: module.lessons.map((lessonPlan) => ({
			id: lessonPlan.id,
			module_id: module.id,
			title: lessonPlan.title,
			position: lessonPlan.position,
			estimated_minutes: lessonPlan.estimatedMinutes,
			is_optional: lessonPlan.isOptional ?? false,
			has_video: false,
		})),
	}));
}

export async function UniversityDebugView({
	view,
}: {
	view: "catalog" | "course" | "lesson" | "broken";
}) {
	const plans = await Promise.all(PLAN_PATHS.map(readPlan));
	const courses = await Promise.all(plans.map(courseListItem));
	const lesson = await welcomeLesson(plans[0], view === "broken");
	const previewPlan = plans[COURSE_PREVIEW_INDEX] ?? plans[0];

	return (
		<UniversityVisualPreview
			view={view}
			courses={courses}
			lesson={lesson.lesson}
			assets={lesson.assets}
			courseDetail={await courseDetail(previewPlan)}
			modules={courseModules(previewPlan)}
		/>
	);
}

export default function UniversityDebugPage() {
	return <UniversityDebugView view="catalog" />;
}
