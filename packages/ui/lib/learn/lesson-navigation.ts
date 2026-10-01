interface OrderedLesson {
	readonly id: string;
	readonly is_optional: boolean;
}

/** Required lessons stay on the core path; electives are opened deliberately. */
export function lessonNeighbors<T extends OrderedLesson>(
	lessons: readonly T[],
	currentId: string,
): { previous: T | null; next: T | null } {
	const current = lessons.find((lesson) => lesson.id === currentId);
	if (!current) return { previous: null, next: null };
	const path = current.is_optional
		? lessons
		: lessons.filter((lesson) => !lesson.is_optional);
	const index = path.findIndex((lesson) => lesson.id === currentId);
	return {
		previous: path[index - 1] ?? null,
		next: path[index + 1] ?? null,
	};
}

export function remainingLessonMinutes(
	lessons: readonly (OrderedLesson & {
		readonly estimated_minutes?: number | null;
	})[],
	completedIds: ReadonlySet<string>,
): { required: number; optional: number } {
	return lessons.reduce(
		(minutes, lesson) => {
			if (!completedIds.has(lesson.id)) {
				minutes[lesson.is_optional ? "optional" : "required"] +=
					lesson.estimated_minutes ?? 0;
			}
			return minutes;
		},
		{ required: 0, optional: 0 },
	);
}
