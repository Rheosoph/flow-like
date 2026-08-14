import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, test } from "vitest";

import {
	buildUniversityOperations,
	loadUniversityPlan,
	validateUniversityPlan,
} from "../plan";
import { UNIVERSITY_PLAN_SCHEMA } from "../types";

const temporaryDirectories: string[] = [];

afterEach(async () => {
	await Promise.all(
		temporaryDirectories
			.splice(0)
			.map((directory) => rm(directory, { recursive: true, force: true })),
	);
});

async function temporaryDirectory(): Promise<string> {
	const directory = await mkdtemp(join(tmpdir(), "flow-like-university-"));
	temporaryDirectories.push(directory);
	return directory;
}

function validPlan() {
	return {
		schema: UNIVERSITY_PLAN_SCHEMA,
		course: {
			id: "course-basics",
			name: "Flow-Like basics",
			language: "en",
			difficulty: "BEGINNER",
			category: "GETTING_STARTED",
			assets: [] as unknown[],
			modules: [
				{
					id: "module-intro",
					title: "Introduction",
					lessons: [
						{
							id: "lesson-welcome",
							title: "Welcome",
							content: "# Welcome",
						},
						{
							id: "lesson-assessment",
							title: "Final assessment",
							content: "# Assessment",
							finalAssessment: true,
							challenges: [
								{
									id: "challenge-final",
									kind: "SINGLE_CHOICE",
									prompt: "Which answer is correct?",
									payload: {
										options: [
											{ id: "a", label: "A" },
											{ id: "b", label: "B" },
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
	};
}

type RawPlan = ReturnType<typeof validPlan>;

function lesson(plan: RawPlan, index: number): Record<string, unknown> {
	const module = plan.course.modules[0];
	const value = module?.lessons[index];
	if (!value) throw new Error(`Missing test lesson ${index}.`);
	return value as unknown as Record<string, unknown>;
}

function finalChallenge(plan: RawPlan): Record<string, unknown> {
	const challenge = (
		lesson(plan, 1).challenges as Record<string, unknown>[]
	)[0];
	if (!challenge) throw new Error("Missing final test challenge.");
	return challenge;
}

describe("University plan validation", () => {
	test("normalizes a complete course and builds dependency-safe operations", () => {
		const plan = validateUniversityPlan(validPlan());

		expect(plan.course).toMatchObject({
			id: "course-basics",
			language: "en",
			difficulty: "BEGINNER",
			category: "GETTING_STARTED",
			estimatedMinutes: 0,
			isPublished: false,
			assets: [],
			appLinks: [],
		});
		expect(plan.course.modules[0]?.lessons).toMatchObject([
			{
				id: "lesson-welcome",
				position: 0,
				isOptional: false,
				finalAssessment: false,
				challenges: [],
			},
			{
				id: "lesson-assessment",
				position: 1,
				finalAssessment: true,
				challenges: [{ points: 10, position: 0 }],
			},
		]);
		expect(buildUniversityOperations(plan).map(({ type }) => type)).toEqual([
			"upsertCourse",
			"upsertModule",
			"upsertLesson",
			"upsertLesson",
			"upsertChallenge",
		]);
	});

	test("loads relative assets and materializes contentFile text", async () => {
		const directory = await temporaryDirectory();
		const planPath = join(directory, "course.plan.json");
		const contentPath = join(directory, "welcome.md");
		const assetPath = join(directory, "overview.png");
		await writeFile(contentPath, "# Welcome from a file\n");
		await writeFile(assetPath, new Uint8Array([137, 80, 78, 71]));

		const input = validPlan();
		const firstLesson = lesson(input, 0);
		firstLesson.content = undefined;
		firstLesson.contentFile = "welcome.md";
		input.course.assets.push({
			name: "EditorOverview",
			file: "overview.png",
		});
		await writeFile(planPath, JSON.stringify(input));

		const plan = await loadUniversityPlan(planPath);

		expect(plan.course.modules[0]?.lessons[0]).toMatchObject({
			content: "# Welcome from a file\n",
			contentFile: resolve(directory, "welcome.md"),
		});
		expect(plan.course.assets[0]).toMatchObject({
			file: resolve(directory, "overview.png"),
			filename: "overview.png",
			extension: "png",
			mimeType: "image/png",
			kind: "IMAGE",
			size: 4,
			replace: false,
		});
	});

	test("rejects a missing local asset before apply", async () => {
		const directory = await temporaryDirectory();
		const planPath = join(directory, "course.plan.json");
		const input = validPlan();
		input.course.assets.push({
			name: "MissingScreenshot",
			file: "missing.png",
		});
		await writeFile(planPath, JSON.stringify(input));

		await expect(loadUniversityPlan(planPath)).rejects.toThrow(
			"plan.course.assets[0].file could not be read",
		);
	});

	test.each([
		[
			"difficulty",
			(plan: RawPlan) => {
				plan.course.difficulty = "BEGINER";
			},
		],
		[
			"category",
			(plan: RawPlan) => {
				plan.course.category = "TUTORIAL";
			},
		],
		[
			"asset kind",
			(plan: RawPlan) => {
				plan.course.assets.push({
					name: "Shot",
					file: "shot.png",
					kind: "PICTURE",
				});
			},
		],
		[
			"challenge kind",
			(plan: RawPlan) => {
				finalChallenge(plan).kind = "QUIZ";
			},
		],
	] as const)(
		"rejects an unknown %s enum instead of silently defaulting",
		(_label, mutate) => {
			const plan = validPlan();
			mutate(plan);
			expect(() => validateUniversityPlan(plan)).toThrow("must be one of");
		},
	);

	test.each([
		[
			"unknown answer",
			(challenge: Record<string, unknown>) => {
				(challenge.payload as { correct: string[] }).correct = ["missing"];
			},
			"references unknown option id",
		],
		[
			"invalid board predicate",
			(challenge: Record<string, unknown>) => {
				challenge.kind = "BOARD_RIDDLE";
				challenge.payload = {
					appAlias: "starter",
					boardId: "board-main",
					predicates: [{ op: "max_nodes", args: [-1] }],
				};
			},
			"one non-negative integer",
		],
		[
			"empty execute-node package proof",
			(challenge: Record<string, unknown>) => {
				challenge.kind = "EXECUTE_NODE";
				challenge.payload = {
					appId: "app-id",
					boardId: "board-main",
					nodeId: "node-id",
					requiredPackages: [],
				};
			},
			"requiredPackages must not be empty",
		],
	] as const)("rejects %s challenge payloads", (_label, mutate, message) => {
		const plan = validPlan();
		mutate(finalChallenge(plan));
		expect(() => validateUniversityPlan(plan)).toThrow(message);
	});

	test.each([
		[
			"not last",
			(plan: RawPlan) => {
				const module = plan.course.modules[0];
				if (!module) throw new Error("Missing test module.");
				module.lessons.reverse();
			},
			"last lesson",
		],
		[
			"positioned before another lesson",
			(plan: RawPlan) => {
				lesson(plan, 0).position = 10;
				lesson(plan, 1).position = 5;
			},
			"last lesson",
		],
		[
			"optional",
			(plan: RawPlan) => {
				lesson(plan, 1).isOptional = true;
			},
			"must be required",
		],
		[
			"without challenges",
			(plan: RawPlan) => {
				lesson(plan, 1).challenges = [];
			},
			"at least one challenge",
		],
		[
			"declared twice",
			(plan: RawPlan) => {
				lesson(plan, 0).finalAssessment = true;
			},
			"Only one lesson",
		],
	] as const)(
		"rejects a final assessment that is %s",
		(_label, mutate, message) => {
			const plan = validPlan();
			mutate(plan);
			expect(() => validateUniversityPlan(plan)).toThrow(message);
		},
	);

	test("rejects duplicate IDs and sibling positions", () => {
		const duplicateId = validPlan();
		lesson(duplicateId, 1).id = lesson(duplicateId, 0).id;
		expect(() => validateUniversityPlan(duplicateId)).toThrow(
			"Duplicate id lesson-welcome",
		);

		const duplicatePosition = validPlan();
		lesson(duplicatePosition, 0).position = 3;
		lesson(duplicatePosition, 1).position = 3;
		expect(() => validateUniversityPlan(duplicatePosition)).toThrow(
			"Duplicate position 3",
		);
	});

	test("rejects unknown fields to catch plan typos", () => {
		const input = validPlan();
		(input.course as unknown as Record<string, unknown>).is_publishd = true;
		expect(() => validateUniversityPlan(input)).toThrow(
			"plan.course.is_publishd is not supported",
		);
	});

	test.each([
		["iconUrl", "media.icon"],
		["bannerUrl", "media.banner"],
	] as const)("rejects unsupported %s values", (field, replacement) => {
		const input = validPlan();
		(input.course as unknown as Record<string, unknown>)[field] =
			"https://cdn.example/image.png";
		expect(() => validateUniversityPlan(input)).toThrow(replacement);
	});

	test("accepts learner navigation modes and rejects inert app references", () => {
		const input = validPlan();
		(input.course as unknown as Record<string, unknown>).appLinks = [
			{ appId: "source-app", alias: "starter" },
		];
		lesson(input, 0).appRefs = [
			{
				kind: "NAVIGATE",
				appAlias: "starter",
				target: { subpath: "flow", params: { id: "board-main" } },
			},
		];
		expect(
			validateUniversityPlan(input).course.modules[0]?.lessons[0]?.appRefs,
		).toHaveLength(1);

		lesson(input, 0).appRefs = [
			{ kind: "NAVIGATE", target: { subpath: "flow" } },
		];
		expect(() => validateUniversityPlan(input)).toThrow(
			"requires appAlias or appId",
		);
	});

	test("rejects contradictory board riddles", () => {
		const input = validPlan();
		const challenge = finalChallenge(input);
		challenge.kind = "BOARD_RIDDLE";
		challenge.payload = {
			appId: "app-id",
			boardId: "board-id",
			predicates: [
				{ op: "requires_nodes", args: ["node-type"] },
				{ op: "forbids_nodes", args: ["node-type"] },
			],
		};
		expect(() => validateUniversityPlan(input)).toThrow(
			"both require and forbid",
		);

		challenge.payload = {
			appId: "app-id",
			boardId: "board-id",
			predicates: [
				{ op: "min_nodes", args: [5] },
				{ op: "max_nodes", args: [2] },
			],
		};
		expect(() => validateUniversityPlan(input)).toThrow(
			"min_nodes above max_nodes",
		);
	});

	test("blocks placeholder or incomplete courses from publication", async () => {
		const directory = await temporaryDirectory();
		const path = join(directory, "course.plan.json");
		const input = validPlan();
		const course = input.course as unknown as Record<string, unknown>;
		course.isPublished = true;
		course.estimatedMinutes = 15;
		course.description = "A practical introduction to Flow-Like.";
		course.longDescription =
			"Learn the core concepts, apply them in a guided lesson, and verify your understanding in the final assessment.";
		lesson(input, 0).content =
			"# New lesson\n\nStart writing. This deliberately includes enough surrounding words to prove that length alone must not allow placeholder material into the published University catalog. Learners deserve complete guidance.";
		lesson(input, 1).content =
			"# Final assessment\n\nReview the learning objectives, work through each scenario carefully, and use the explanations after submitting to close any remaining knowledge gaps before completing this course.";
		await writeFile(path, JSON.stringify(input));

		await expect(loadUniversityPlan(path)).rejects.toThrow(
			"still contains placeholder content",
		);

		lesson(input, 0).content =
			"# Welcome\n\nFlow-Like models application behavior as explicit, typed graphs. In this lesson you will identify the application boundary, trace execution through connected nodes, inspect the resulting run, and explain how the graph turns inputs into repeatable outcomes.";
		lesson(input, 1).finalAssessment = false;
		await writeFile(path, JSON.stringify(input));
		await expect(loadUniversityPlan(path)).rejects.toThrow(
			"require one finalAssessment lesson",
		);
	});
});
