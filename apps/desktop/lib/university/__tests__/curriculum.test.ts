import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import {
	CURRICULUM_SCHEMA,
	checkCurriculum,
	validateCurriculum,
} from "../curriculum";
import { UNIVERSITY_PLAN_SCHEMA } from "../types";

const directories: string[] = [];

afterEach(async () => {
	await Promise.all(
		directories
			.splice(0)
			.map((directory) => rm(directory, { recursive: true, force: true })),
	);
});

function catalog(
	core = ["courses/core/first/course.plan.json"],
	electives: string[] = [],
) {
	return {
		schema: CURRICULUM_SCHEMA,
		paths: [
			{
				id: "path-first",
				slug: "first",
				name: "Build a workflow",
				description: "Run a small workflow.",
				courses: core,
				electives,
			},
		],
	};
}

function course(id: string, optional = true) {
	return {
		schema: UNIVERSITY_PLAN_SCHEMA,
		course: {
			id,
			name: id,
			slug: id,
			estimatedMinutes: 5,
			modules: [
				{
					id: `${id}-module`,
					title: "Practice",
					lessons: [
						{
							id: `${id}-lesson`,
							title: "Run",
							content: "Run the workflow.",
							estimatedMinutes: 5,
						},
						...(optional
							? [
									{
										id: `${id}-optional`,
										title: "Extension",
										content: "Try another input.",
										estimatedMinutes: 3,
										isOptional: true,
									},
								]
							: []),
					],
				},
			],
		},
	};
}

async function fixture(value = catalog()) {
	const directory = await mkdtemp(join(tmpdir(), "university-curriculum-"));
	directories.push(directory);
	const path = join(directory, "curriculum.json");
	await writeFile(path, JSON.stringify(value));
	return { directory, path };
}

async function save(directory: string, reference: string, plan: unknown) {
	const path = join(directory, reference);
	await mkdir(join(path, ".."), { recursive: true });
	await writeFile(path, JSON.stringify(plan));
}

describe("University curriculum", () => {
	test("computes required and optional durations separately for courses and paths", async () => {
		const core = "courses/core/first/course.plan.json";
		const elective = "courses/specialist/extra/course.plan.json";
		const { directory, path } = await fixture(catalog([core], [elective]));
		await save(directory, core, course("first"));
		await save(directory, elective, course("extra", false));
		const result = await checkCurriculum(path);
		expect(result.passed).toBe(true);
		expect(result.totals).toMatchObject({
			courses: 2,
			paths: 1,
			requiredMinutes: 10,
			optionalMinutes: 3,
			lessons: 3,
			optionalLessons: 1,
			lessonWords: 9,
		});
		expect(result.paths[0]).toMatchObject({
			courseIds: ["first"],
			electiveIds: ["extra"],
			requiredMinutes: 5,
			optionalLessonMinutes: 3,
			electiveMinutes: 5,
		});
	});

	test("checks discovered plans even when no learning path references them", async () => {
		const { directory, path } = await fixture();
		await save(
			directory,
			"courses/core/first/course.plan.json",
			course("first"),
		);
		const broken = { ...course("hidden"), unexpected: true };
		await save(directory, "courses/unlisted/hidden/course.plan.json", broken);
		const result = await checkCurriculum(path);
		expect(result.passed).toBe(false);
		expect(
			result.errors.some(
				(error) =>
					error.includes("unlisted/hidden") && error.includes("unexpected"),
			),
		).toBe(true);
	});

	test("rejects duplicate IDs across otherwise valid course plans", async () => {
		const { directory, path } = await fixture();
		const first = course("first");
		const other = course("other");
		other.course.modules[0].lessons[0].id =
			first.course.modules[0].lessons[0].id;
		await save(directory, "courses/core/first/course.plan.json", first);
		await save(directory, "courses/core/other/course.plan.json", other);
		const result = await checkCurriculum(path);
		expect(
			result.errors.some((error) =>
				error.includes("Duplicate entity id first-lesson"),
			),
		).toBe(true);
	});

	test("rejects duplicate course IDs and slugs", async () => {
		const { directory, path } = await fixture();
		await save(
			directory,
			"courses/core/first/course.plan.json",
			course("first"),
		);
		const other = course("other");
		other.course.id = "first";
		other.course.slug = "first";
		await save(directory, "courses/core/other/course.plan.json", other);
		const result = await checkCurriculum(path);
		expect(
			result.errors.some((error) =>
				error.startsWith("Duplicate entity id first:"),
			),
		).toBe(true);
		expect(
			result.errors.some((error) =>
				error.startsWith("Duplicate course slug first:"),
			),
		).toBe(true);
	});

	test("rejects advertised time that includes an optional lesson", async () => {
		const { directory, path } = await fixture();
		const plan = course("first");
		plan.course.estimatedMinutes = 8;
		await save(directory, "courses/core/first/course.plan.json", plan);
		const result = await checkCurriculum(path);
		expect(
			result.errors.some((error) =>
				error.includes("required lessons sum to 5"),
			),
		).toBe(true);
	});

	test("loads referenced content and assets through the course validator", async () => {
		const { directory, path } = await fixture();
		const plan = {
			...course("first"),
			course: {
				...course("first").course,
				assets: [{ name: "Missing", file: "missing.txt" }],
			},
		};
		await save(directory, "courses/core/first/course.plan.json", plan);
		const result = await checkCurriculum(path);
		expect(result.errors.some((error) => error.includes("missing.txt"))).toBe(
			true,
		);
		expect(
			result.errors.some((error) => error.includes("missing or invalid plan")),
		).toBe(true);
	});

	test("reports missing path references", async () => {
		const { directory, path } = await fixture(
			catalog(["courses/core/missing/course.plan.json"]),
		);
		await save(
			directory,
			"courses/core/first/course.plan.json",
			course("first"),
		);
		const result = await checkCurriculum(path);
		expect(result.errors).toContain(
			"Path path-first references a missing or invalid plan: courses/core/missing/course.plan.json.",
		);
	});

	test("rejects traversal, duplicate steps, and overlapping electives", () => {
		expect(() =>
			validateCurriculum(catalog(["../outside/course.plan.json"])),
		).toThrow("under courses/");
		const reference = "courses/core/first/course.plan.json";
		expect(() => validateCurriculum(catalog([reference, reference]))).toThrow(
			"repeats",
		);
		expect(() => validateCurriculum(catalog([reference], [reference]))).toThrow(
			"both required and elective",
		);
	});

	test("rejects duplicate path IDs and unsupported catalog fields", () => {
		const input = catalog();
		input.paths.push({ ...input.paths[0], slug: "second" });
		expect(() => validateCurriculum(input)).toThrow(
			"Duplicate learning path id",
		);
		expect(() =>
			validateCurriculum({ ...catalog(), courseRoot: "/elsewhere" }),
		).toThrow("courseRoot is not supported");
	});
});
