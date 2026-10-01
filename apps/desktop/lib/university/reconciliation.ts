import type { UniversityClient } from "./client";
import type { UniversityPlan } from "./types";

export interface UniversityRetirement {
	kind: "module" | "lesson" | "challenge" | "appRef" | "appLink";
	id: string;
	moduleId?: string;
	lessonId?: string;
}

function record(value: unknown): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("Retirement inspection returned an invalid object.");
	}
	return value as Record<string, unknown>;
}

function rows(value: unknown): Record<string, unknown>[] {
	if (!Array.isArray(value)) {
		throw new Error("Retirement inspection returned an invalid collection.");
	}
	return value.map(record);
}

function id(value: unknown): string {
	if (typeof value !== "string" || !value) {
		throw new Error("Retirement inspection returned an invalid ID.");
	}
	return value;
}

function expectParent(actual: unknown, expected: string, entity: string): void {
	if (actual !== expected) {
		throw new Error(
			`Cannot retire content: ${entity} has an unexpected parent.`,
		);
	}
}

/** Inspect every descendant before deleting a parent that cascades to its children. */
export async function findUniversityRetirements(
	client: UniversityClient,
	plan: UniversityPlan,
): Promise<UniversityRetirement[]> {
	const courseId = plan.course.id;
	const structure = record(
		await client.getCourseStructure(courseId, plan.course.language),
	);
	const course = record(structure.course);
	if (course.id !== courseId || course.is_published !== false) {
		throw new Error(
			"Cannot retire content until the target course is confirmed as a draft.",
		);
	}
	const expectedModules = new Set(
		plan.course.modules.map((module) => module.id),
	);
	const expectedLessons = new Map(
		plan.course.modules.flatMap((module) =>
			module.lessons.map((lesson) => [lesson.id, module.id] as const),
		),
	);
	const expectedChallenges = new Map(
		plan.course.modules.flatMap((module) =>
			module.lessons.flatMap((lesson) =>
				lesson.challenges.map(
					(challenge) => [challenge.id, lesson.id] as const,
				),
			),
		),
	);
	const expectedRefs = new Map(
		plan.course.modules.flatMap((module) =>
			module.lessons.flatMap((lesson) =>
				lesson.appRefs.map((ref) => [ref.id, lesson.id] as const),
			),
		),
	);
	const retirements: UniversityRetirement[] = [];
	const seen = new Set<string>();
	function unique(kind: string, entityId: string): void {
		const key = `${kind}:${entityId}`;
		if (seen.has(key)) throw new Error(`Duplicate remote ${kind} ${entityId}.`);
		seen.add(key);
	}
	for (const module of rows(structure.modules)) {
		const moduleId = id(module.id);
		unique("module", moduleId);
		expectParent(module.course_id, courseId, `module ${moduleId}`);
		const retireModule = !expectedModules.has(moduleId);
		for (const summary of rows(module.lessons)) {
			const lessonId = id(summary.id);
			unique("lesson", lessonId);
			const expectedModuleId = expectedLessons.get(lessonId);
			if (expectedModuleId && expectedModuleId !== moduleId) {
				throw new Error(
					`Cannot retire module ${moduleId}: retained lesson ${lessonId} needs an explicit parent migration.`,
				);
			}
			const retireLesson = !expectedModuleId;
			const detail = record(
				await client.getLesson(courseId, moduleId, lessonId),
			);
			const lesson = record(detail.lesson);
			if (lesson.id !== lessonId)
				throw new Error(`Lesson ${lessonId} readback returned a different ID.`);
			expectParent(lesson.module_id, moduleId, `lesson ${lessonId}`);
			for (const [kind, values, expectedParents] of [
				["challenge", detail.challenges, expectedChallenges],
				["appRef", detail.app_refs, expectedRefs],
			] as const) {
				for (const child of rows(values)) {
					const childId = id(child.id);
					expectParent(child.lesson_id, lessonId, `${kind} ${childId}`);
					const expectedLessonId = expectedParents.get(childId);
					if (expectedLessonId && expectedLessonId !== lessonId) {
						throw new Error(
							`Cannot retire lesson ${lessonId}: retained ${kind} ${childId} needs an explicit parent migration.`,
						);
					}
					unique(kind, childId);
					if (!expectedLessonId && !retireModule && !retireLesson) {
						retirements.push({ kind, id: childId, moduleId, lessonId });
					}
				}
			}
			if (retireLesson && !retireModule) {
				retirements.push({ kind: "lesson", id: lessonId, moduleId });
			}
		}
		if (retireModule) retirements.push({ kind: "module", id: moduleId });
	}
	const expectedLinks = new Set(plan.course.appLinks.map((link) => link.id));
	for (const link of rows(await client.listAppLinks(courseId))) {
		const linkId = id(link.id);
		unique("appLink", linkId);
		expectParent(link.course_id, courseId, `application link ${linkId}`);
		if (!expectedLinks.has(linkId))
			retirements.push({ kind: "appLink", id: linkId });
	}
	for (const [kind, expected] of [
		["module", expectedModules],
		["lesson", expectedLessons.keys()],
		["challenge", expectedChallenges.keys()],
		["appRef", expectedRefs.keys()],
		["appLink", expectedLinks],
	] as const) {
		for (const entityId of expected) {
			if (!seen.has(`${kind}:${entityId}`)) {
				throw new Error(
					`Cannot retire content: expected ${kind} ${entityId} is missing after upsert.`,
				);
			}
		}
	}
	return retirements;
}

export async function retireUniversityEntity(
	client: UniversityClient,
	courseId: string,
	retirement: UniversityRetirement,
): Promise<void> {
	switch (retirement.kind) {
		case "module":
			return client.deleteModule(courseId, retirement.id, true);
		case "lesson":
			return client.deleteLesson(
				courseId,
				id(retirement.moduleId),
				retirement.id,
				true,
			);
		case "challenge":
			return client.deleteChallenge(
				courseId,
				id(retirement.lessonId),
				retirement.id,
				true,
			);
		case "appRef":
			return client.deleteAppRef(
				courseId,
				id(retirement.lessonId),
				retirement.id,
			);
		case "appLink":
			return client.deleteAppLink(courseId, retirement.id);
	}
}
