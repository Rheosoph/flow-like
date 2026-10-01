import { execFile } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { describe, expect, test } from "vitest";
import { compareCourseEntities, compareCurriculumRef } from "../migration";
import { validateUniversityPlan } from "../plan";

function snapshot(
	moduleId = "module-a",
	lessonId = "lesson-a",
	challengeId = "challenge-a",
	plan = "courses/example/course.plan.json",
) {
	return {
		plan,
		value: validateUniversityPlan({
			schema: "flow-like.university-plan/v1",
			course: {
				id: "course-a",
				name: "Course",
				modules: [
					{
						id: moduleId,
						title: "Practice",
						lessons: [
							{
								id: lessonId,
								title: "Lesson",
								content: "Practice.",
								challenges: [
									{
										id: challengeId,
										kind: "SINGLE_CHOICE",
										prompt: "Pick one.",
										payload: {
											options: [
												{ id: "a", label: "One" },
												{ id: "b", label: "Two" },
											],
											correct: ["a"],
										},
									},
								],
							},
						],
					},
				],
			},
		}),
	};
}

describe("course migration comparison", () => {
	test("lists obsolete children without treating new IDs as moves", () => {
		const before = snapshot();
		const after = snapshot("module-a", "lesson-b", "challenge-b");
		const result = compareCourseEntities([before], [after]);
		expect(result.removed.map((entity) => [entity.kind, entity.id])).toEqual([
			["challenge", "challenge-a"],
			["lesson", "lesson-a"],
		]);
		expect(result.moved).toEqual([]);
	});

	test("reports retained IDs attached to a different parent", () => {
		const result = compareCourseEntities([snapshot()], [snapshot("module-b")]);
		expect(result.removed.map((entity) => entity.id)).toEqual(["module-a"]);
		expect(
			result.moved.find((entity) => entity.id === "lesson-a"),
		).toMatchObject({
			from: { moduleId: "module-a" },
			to: { moduleId: "module-b" },
		});
		expect(
			result.moved.find((entity) => entity.id === "challenge-a"),
		).toMatchObject({
			from: { moduleId: "module-a", lessonId: "lesson-a" },
			to: { moduleId: "module-b", lessonId: "lesson-a" },
		});
	});

	test("a source file move alone does not require a remote entity move", () => {
		const result = compareCourseEntities(
			[snapshot()],
			[
				snapshot(
					"module-a",
					"lesson-a",
					"challenge-a",
					"courses/renamed/course.plan.json",
				),
			],
		);
		expect(result).toEqual({ removed: [], moved: [] });
	});

	test("refuses ambiguous duplicate IDs", () => {
		expect(() => compareCourseEntities([snapshot(), snapshot()], [])).toThrow(
			"duplicate entity id module-a",
		);
	});
});

test("reads historical plans from a catalog nested inside a Git repository", async () => {
	const directory = await mkdtemp(join(tmpdir(), "university-migration-"));
	const run = promisify(execFile);
	try {
		const base = join(directory, "apps", "desktop", "lib", "university");
		const coursePath = join(base, "courses", "core", "example");
		await mkdir(coursePath, { recursive: true });
		const planPath = join(coursePath, "course.plan.json");
		await writeFile(planPath, JSON.stringify(snapshot().value));
		await run("git", ["init", "-q"], { cwd: directory });
		await run("git", ["add", "."], { cwd: directory });
		await run(
			"git",
			[
				"-c",
				"user.name=Course Test",
				"-c",
				"user.email=course-test@example.invalid",
				"-c",
				"commit.gpgsign=false",
				"-c",
				"core.hooksPath=/dev/null",
				"commit",
				"-qm",
				"baseline",
			],
			{ cwd: directory },
		);
		await writeFile(
			planPath,
			JSON.stringify(snapshot("module-a", "lesson-new", "challenge-new").value),
		);
		const result = await compareCurriculumRef(
			join(base, "curriculum.json"),
			"HEAD",
		);
		expect(result.removed.map((entity) => entity.id)).toEqual([
			"challenge-a",
			"lesson-a",
		]);
		expect(result.removed[0].location.plan).toBe(
			"apps/desktop/lib/university/courses/core/example/course.plan.json",
		);
		expect(result.commit).toMatch(/^[a-f0-9]{40}$/);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
});
