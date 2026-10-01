import { readFile, readdir } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { loadUniversityPlan } from "./plan";
import type { UniversityCoursePlan, UniversityPlan } from "./types";

export const CURRICULUM_SCHEMA = "flow-like.university-curriculum/v1";

export interface LearningPathPlan {
	id: string;
	slug: string;
	name: string;
	description: string;
	courses: string[];
	electives: string[];
}

export interface CurriculumPlan {
	schema: typeof CURRICULUM_SCHEMA;
	paths: LearningPathPlan[];
}

export interface CourseCheckSummary {
	plan: string;
	id: string;
	name: string;
	requiredMinutes: number;
	optionalMinutes: number;
	lessons: number;
	optionalLessons: number;
	questions: number;
	interactiveChallenges: number;
	lessonWords: number;
	questionWords: number;
}

export interface CurriculumCheckResult {
	schema: "flow-like.university-check/v1";
	passed: boolean;
	errors: string[];
	courses: CourseCheckSummary[];
	paths: Array<{
		id: string;
		name: string;
		courseIds: string[];
		electiveIds: string[];
		requiredMinutes: number;
		optionalLessonMinutes: number;
		electiveMinutes: number;
	}>;
	totals: Omit<CourseCheckSummary, "plan" | "id" | "name"> & {
		courses: number;
		paths: number;
	};
}

function object(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new Error(`${label} must be an object.`);
	return value as Record<string, unknown>;
}

function keys(
	value: Record<string, unknown>,
	allowed: string[],
	label: string,
) {
	for (const key of Object.keys(value))
		if (!allowed.includes(key))
			throw new Error(`${label}.${key} is not supported.`);
}

function text(value: unknown, label: string): string {
	if (typeof value !== "string" || !value.trim())
		throw new Error(`${label} must be a non-empty string.`);
	return value;
}

function planReferences(
	value: unknown,
	label: string,
	required: boolean,
): string[] {
	if (!Array.isArray(value) || (required && !value.length))
		throw new Error(
			`${label} must be ${required ? "a non-empty" : "an"} array.`,
		);
	const seen = new Set<string>();
	return value.map((item, index) => {
		const path = text(item, `${label}[${index}]`);
		if (
			isAbsolute(path) ||
			path.includes("\\") ||
			!/^courses\/(?:[a-z0-9-]+\/)+course\.plan\.json$/.test(path)
		)
			throw new Error(
				`${label}[${index}] must be a course plan path under courses/.`,
			);
		if (seen.has(path)) throw new Error(`${label} repeats ${path}.`);
		seen.add(path);
		return path;
	});
}

export function validateCurriculum(value: unknown): CurriculumPlan {
	const input = object(value, "curriculum");
	keys(input, ["schema", "paths"], "curriculum");
	if (input.schema !== CURRICULUM_SCHEMA)
		throw new Error(`curriculum.schema must be ${CURRICULUM_SCHEMA}.`);
	if (!Array.isArray(input.paths) || !input.paths.length)
		throw new Error("curriculum.paths must be a non-empty array.");
	const ids = new Set<string>();
	const slugs = new Set<string>();
	const paths = input.paths.map((raw, index) => {
		const label = `curriculum.paths[${index}]`;
		const path = object(raw, label);
		keys(
			path,
			["id", "slug", "name", "description", "courses", "electives"],
			label,
		);
		const id = text(path.id, `${label}.id`);
		const slug = text(path.slug, `${label}.slug`);
		if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(id))
			throw new Error(`${label}.id is not a safe identifier.`);
		if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(slug))
			throw new Error(`${label}.slug is not a lowercase slug.`);
		if (ids.has(id)) throw new Error(`Duplicate learning path id ${id}.`);
		if (slugs.has(slug))
			throw new Error(`Duplicate learning path slug ${slug}.`);
		ids.add(id);
		slugs.add(slug);
		const courses = planReferences(path.courses, `${label}.courses`, true);
		const electives = planReferences(
			path.electives ?? [],
			`${label}.electives`,
			false,
		);
		for (const elective of electives)
			if (courses.includes(elective))
				throw new Error(
					`${label} lists ${elective} as both required and elective.`,
				);
		return {
			id,
			slug,
			name: text(path.name, `${label}.name`),
			description: text(path.description, `${label}.description`),
			courses,
			electives,
		};
	});
	return { schema: CURRICULUM_SCHEMA, paths };
}

export async function discoverCoursePlans(
	directory: string,
): Promise<string[]> {
	const entries = await readdir(directory, { withFileTypes: true });
	const nested = await Promise.all(
		entries.map(async (entry) => {
			const path = join(directory, entry.name);
			if (entry.isDirectory()) return discoverCoursePlans(path);
			return entry.isFile() && entry.name === "course.plan.json" ? [path] : [];
		}),
	);
	return nested.flat().sort();
}

function wordCount(value: string): number {
	return value.trim() ? value.trim().split(/\s+/u).length : 0;
}

function summarize(
	course: UniversityCoursePlan,
	path: string,
): CourseCheckSummary {
	const lessons = course.modules.flatMap((module) => module.lessons);
	const challenges = lessons.flatMap((lesson) => lesson.challenges);
	const questions = challenges.filter((challenge) =>
		challenge.kind.endsWith("CHOICE"),
	);
	return {
		plan: path,
		id: course.id,
		name: course.name,
		requiredMinutes: lessons.reduce(
			(sum, lesson) => sum + (lesson.isOptional ? 0 : lesson.estimatedMinutes),
			0,
		),
		optionalMinutes: lessons.reduce(
			(sum, lesson) => sum + (lesson.isOptional ? lesson.estimatedMinutes : 0),
			0,
		),
		lessons: lessons.length,
		optionalLessons: lessons.filter((lesson) => lesson.isOptional).length,
		questions: questions.length,
		interactiveChallenges: challenges.length - questions.length,
		lessonWords: lessons.reduce(
			(sum, lesson) => sum + wordCount(lesson.content),
			0,
		),
		questionWords: questions.reduce((sum, question) => {
			const payload = question.payload as { options: Array<{ label: string }> };
			return (
				sum +
				wordCount(
					[
						question.prompt,
						question.explanation ?? "",
						...payload.options.map((option) => option.label),
					].join(" "),
				)
			);
		}, 0),
	};
}

function entityIds(plan: UniversityPlan): string[] {
	const course = plan.course;
	return [
		course.id,
		...course.appLinks.map((link) => link.id),
		...course.modules.flatMap((module) => [
			module.id,
			...module.lessons.flatMap((lesson) => [
				lesson.id,
				...lesson.challenges.map((challenge) => challenge.id),
				...lesson.appRefs.map((ref) => ref.id),
			]),
		]),
	];
}

export async function checkCurriculum(
	catalogPath: string,
): Promise<CurriculumCheckResult> {
	const catalog = resolve(catalogPath);
	const base = dirname(catalog);
	const curriculum = validateCurriculum(
		JSON.parse(await readFile(catalog, "utf8")),
	);
	const files = await discoverCoursePlans(join(base, "courses"));
	const errors: string[] = [];
	if (!files.length)
		errors.push("No course.plan.json files found under courses/.");
	const loaded = await Promise.allSettled(
		files.map((path) => loadUniversityPlan(path)),
	);
	const ids = new Map<string, string>();
	const slugs = new Map<string, string>();
	const byPath = new Map<string, CourseCheckSummary>();
	for (const [index, result] of loaded.entries()) {
		const path = relative(base, files[index]).split(sep).join("/");
		if (result.status === "rejected") {
			errors.push(
				`${path}: ${result.reason instanceof Error ? result.reason.message : String(result.reason)}`,
			);
			continue;
		}
		const plan = result.value;
		for (const id of entityIds(plan)) {
			const previous = ids.get(id);
			if (previous)
				errors.push(`Duplicate entity id ${id}: ${previous} and ${path}.`);
			else ids.set(id, path);
		}
		if (plan.course.slug) {
			const previous = slugs.get(plan.course.slug);
			if (previous)
				errors.push(
					`Duplicate course slug ${plan.course.slug}: ${previous} and ${path}.`,
				);
			else slugs.set(plan.course.slug, path);
		}
		const summary = summarize(plan.course, path);
		if (summary.requiredMinutes !== plan.course.estimatedMinutes)
			errors.push(
				`${path}: course estimatedMinutes is ${plan.course.estimatedMinutes}; required lessons sum to ${summary.requiredMinutes}.`,
			);
		byPath.set(path, summary);
	}
	const paths = curriculum.paths.map((path) => {
		for (const reference of [...path.courses, ...path.electives])
			if (!byPath.has(reference))
				errors.push(
					`Path ${path.id} references a missing or invalid plan: ${reference}.`,
				);
		const core = path.courses.flatMap(
			(reference) => byPath.get(reference) ?? [],
		);
		const electives = path.electives.flatMap(
			(reference) => byPath.get(reference) ?? [],
		);
		return {
			id: path.id,
			name: path.name,
			courseIds: core.map((course) => course.id),
			electiveIds: electives.map((course) => course.id),
			requiredMinutes: core.reduce(
				(sum, course) => sum + course.requiredMinutes,
				0,
			),
			optionalLessonMinutes: core.reduce(
				(sum, course) => sum + course.optionalMinutes,
				0,
			),
			electiveMinutes: electives.reduce(
				(sum, course) => sum + course.requiredMinutes,
				0,
			),
		};
	});
	const courses = [...byPath.values()];
	const totals = {
		courses: courses.length,
		paths: paths.length,
		requiredMinutes: 0,
		optionalMinutes: 0,
		lessons: 0,
		optionalLessons: 0,
		questions: 0,
		interactiveChallenges: 0,
		lessonWords: 0,
		questionWords: 0,
	};
	for (const course of courses)
		for (const key of [
			"requiredMinutes",
			"optionalMinutes",
			"lessons",
			"optionalLessons",
			"questions",
			"interactiveChallenges",
			"lessonWords",
			"questionWords",
		] as const)
			totals[key] += course[key];
	return {
		schema: "flow-like.university-check/v1",
		passed: errors.length === 0,
		errors,
		courses,
		paths,
		totals,
	};
}
