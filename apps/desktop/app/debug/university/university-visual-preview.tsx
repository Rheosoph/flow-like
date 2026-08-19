"use client";

import {
	CourseCatalog,
	CourseDetailView,
	LessonContent,
} from "@flow-like/flow-like-ui/components/learn";
import type {
	CourseDetail,
	CourseListItem,
	Lesson,
	LessonAssetView,
	ModuleWithLessons,
} from "@flow-like/flow-like-ui/lib/learn/types";

interface UniversityVisualPreviewProps {
	readonly view: "catalog" | "course" | "lesson" | "broken";
	readonly courses: ReadonlyArray<CourseListItem>;
	readonly lesson: Lesson;
	readonly assets: ReadonlyArray<LessonAssetView>;
	readonly courseDetail?: CourseDetail;
	readonly modules?: ReadonlyArray<ModuleWithLessons>;
}

const noop = () => {};

export function UniversityVisualPreview({
	view,
	courses,
	lesson,
	assets,
	courseDetail,
	modules = [],
}: UniversityVisualPreviewProps) {
	if (view === "catalog") {
		return (
			<main
				data-university-visual="catalog"
				className="min-h-screen overflow-y-auto bg-background px-5 py-8 md:px-8 lg:px-10"
			>
				<div className="mx-auto max-w-[94rem]">
					<CourseCatalog courses={courses} displayName="Builder" />
				</div>
			</main>
		);
	}

	if (view === "course" && courseDetail) {
		/* Part-way through module one, so the spine shows done / next / upcoming. */
		const completed = new Set(modules[0]?.lessons.slice(0, 2).map((l) => l.id));
		return (
			<main
				data-university-visual="course"
				className="flex min-h-screen flex-col overflow-y-auto bg-background"
			>
				<CourseDetailView
					courseId={courseDetail.id}
					course={courseDetail}
					modules={modules}
					completedLessonIds={completed}
					isEnrolled
					workspaceAppId="preview-app"
					onBack={noop}
					onEnroll={noop}
					onOpenLesson={noop}
					onClaimCertificate={noop}
					onOpenWorkspace={noop}
				/>
			</main>
		);
	}

	return (
		<main
			data-university-visual={view}
			className="min-h-screen overflow-y-auto bg-background px-5 py-10 md:px-10 md:py-14"
		>
			<LessonContent lesson={lesson} assets={assets} />
		</main>
	);
}
