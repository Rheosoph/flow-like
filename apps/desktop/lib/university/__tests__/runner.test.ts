import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test, vi } from "vitest";

import { validateUniversityPlan } from "../plan";
import {
	inspectUniversityCourse,
	listUniversityCourses,
	planUniversityRun,
	runUniversityPlan,
	uploadSingleUniversityAsset,
} from "../runner";
import { UNIVERSITY_PLAN_SCHEMA } from "../types";

const originalFetch = globalThis.fetch;
const temporaryDirectories: string[] = [];

// Bun's vitest shim has no stubGlobal/unstubAllGlobals, and the Bun global is
// non-configurable there, so the two tests that must stub Bun.file only run
// under vitest. Everything else restores fetch by hand.
const canStubGlobals = typeof vi.stubGlobal === "function";
const stubBunFile = (): void => {
	vi.stubGlobal("Bun", {
		file: (_path: string, options?: BlobPropertyBag) =>
			new Blob([new Uint8Array([1, 2, 3, 4])], options),
	});
};

afterEach(async () => {
	globalThis.fetch = originalFetch;
	vi.restoreAllMocks();
	vi.unstubAllGlobals?.();
	await Promise.all(
		temporaryDirectories
			.splice(0)
			.map((directory) => rm(directory, { recursive: true, force: true })),
	);
});

function authoredPlan(isPublished: boolean) {
	return validateUniversityPlan({
		schema: UNIVERSITY_PLAN_SCHEMA,
		course: {
			id: "course-basics",
			name: "Flow-Like basics",
			isPublished,
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
					],
				},
			],
		},
	});
}

const remoteOptions = {
	apiUrl: "https://flow.example",
	pat: "pat_test.secret",
	timeoutMs: 5_000,
};

function jsonResponse(value: unknown, status = 200): Response {
	return new Response(JSON.stringify(value), {
		status,
		headers: { "Content-Type": "application/json" },
	});
}

describe("University runner", () => {
	test("plans a dry run without making an API request", () => {
		const fetchSpy = vi.fn(() => {
			throw new Error("dry run must not fetch");
		});
		globalThis.fetch = fetchSpy as unknown as typeof fetch;

		const result = planUniversityRun(authoredPlan(false));

		expect(fetchSpy).not.toHaveBeenCalled();
		expect(result).toMatchObject({
			command: "plan",
			passed: true,
			courseId: "course-basics",
		});
		expect(
			result.operations?.map(({ type, status }) => [type, status]),
		).toEqual([
			["upsertCourse", "planned"],
			["upsertModule", "planned"],
			["upsertLesson", "planned"],
			["verify", "planned"],
		]);
	});

	test("orders verification before the final publication write", () => {
		const result = planUniversityRun(authoredPlan(true));

		expect(
			result.operations?.map(({ type, description }) => [type, description]),
		).toEqual([
			["upsertCourse", "Upsert draft course course-basics"],
			["upsertModule", "Upsert module module-intro"],
			["upsertLesson", "Upsert lesson lesson-welcome"],
			["verify", "Verify the remote course structure and authored children"],
			["upsertCourse", "Publish course course-basics"],
		]);
	});

	test("applies every authored body, verifies the remote tree, then publishes", async () => {
		const plan = validateUniversityPlan({
			schema: UNIVERSITY_PLAN_SCHEMA,
			course: {
				id: "course-basics",
				name: "Flow-Like basics",
				language: "en",
				slug: "flow-like-basics",
				difficulty: "INTERMEDIATE",
				category: "FLOWS",
				estimatedMinutes: 25,
				isPublished: true,
				tags: ["basics", "agents"],
				position: 2,
				description: "Build your first Flow-Like workflow.",
				longDescription: "A complete, agent-authored introduction.",
				modules: [
					{
						id: "module-intro",
						title: "Introduction",
						description: "Start here.",
						position: 3,
						lessons: [
							{
								id: "lesson-welcome",
								title: "Welcome",
								content: "# Welcome",
								videoUrl: "https://video.example/welcome",
								estimatedMinutes: 7,
								position: 4,
								isOptional: false,
								finalAssessment: true,
								challenges: [
									{
										id: "challenge-choice",
										kind: "SINGLE_CHOICE",
										prompt: "Which answer is correct?",
										explanation: "The first answer is correct.",
										points: 15,
										position: 6,
										payload: {
											options: [
												{ id: "a", label: "First" },
												{ id: "b", label: "Second" },
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
		});
		const remoteCourse = {
			id: "course-basics",
			language: "en",
			slug: "flow-like-basics",
			difficulty: "INTERMEDIATE",
			category: "FLOWS",
			estimated_minutes: 25,
			is_published: false,
			icon_url: null,
			banner_url: null,
			tags: ["basics", "agents"],
			position: 2,
			name: "Flow-Like basics",
			description: "Build your first Flow-Like workflow.",
			long_description: "A complete, agent-authored introduction.",
		};
		const remoteChallenge = {
			id: "challenge-choice",
			lesson_id: "lesson-welcome",
			kind: "SINGLE_CHOICE",
			prompt: "Which answer is correct?",
			explanation: "The first answer is correct.",
			payload: {
				options: [
					{ id: "a", label: "First" },
					{ id: "b", label: "Second" },
				],
				correct: ["a"],
			},
			points: 15,
			position: 6,
		};
		const requests: Array<{
			method: string;
			url: string;
			body: unknown;
		}> = [];
		globalThis.fetch = vi.fn(async (input, init) => {
			const method = init?.method ?? "GET";
			const url = String(input);
			requests.push({
				method,
				url,
				body: init?.body ? JSON.parse(String(init.body)) : undefined,
			});

			const parsedUrl = new URL(url);
			if (method === "GET" && parsedUrl.pathname.endsWith("/structure")) {
				return Response.json({
					course: remoteCourse,
					modules: [
						{
							id: "module-intro",
							course_id: "course-basics",
							title: "Introduction",
							description: "Start here.",
							position: 3,
							lessons: [{ id: "lesson-welcome" }],
						},
					],
				});
			}
			if (method === "GET" && parsedUrl.pathname.endsWith("/lesson-welcome")) {
				return Response.json({
					lesson: {
						id: "lesson-welcome",
						module_id: "module-intro",
						title: "Welcome",
						language: "en",
						content: "# Welcome",
						video_url: "https://video.example/welcome",
						estimated_minutes: 7,
						position: 4,
						is_optional: false,
					},
					challenges: [remoteChallenge],
					app_refs: [],
					attempts: [],
					assets: [],
				});
			}
			if (
				method === "GET" &&
				(parsedUrl.pathname.endsWith("/assets") ||
					parsedUrl.pathname.endsWith("/app-links"))
			) {
				return Response.json([]);
			}
			if (
				method === "PUT" &&
				parsedUrl.pathname === "/api/v1/courses/course-basics"
			) {
				const body = requests.at(-1)?.body as { is_published: boolean };
				return Response.json({
					...remoteCourse,
					is_published: body.is_published,
				});
			}
			return Response.json({ ok: true });
		}) as unknown as typeof fetch;

		const result = await runUniversityPlan(plan, remoteOptions);

		expect(result).toMatchObject({
			command: "apply",
			passed: true,
			courseId: "course-basics",
			summary: "Course applied, verified, and published.",
		});
		expect(
			result.operations?.map(({ type, status }) => [type, status]),
		).toEqual([
			["upsertCourse", "completed"],
			["upsertModule", "completed"],
			["upsertLesson", "completed"],
			["upsertChallenge", "completed"],
			["verify", "completed"],
			["upsertCourse", "completed"],
		]);
		expect(
			requests.map(({ method, url }) => {
				const parsedUrl = new URL(url);
				return `${method} ${parsedUrl.pathname}${parsedUrl.search}`;
			}),
		).toEqual([
			"PUT /api/v1/courses/course-basics",
			"PUT /api/v1/courses/course-basics/modules/module-intro",
			"PUT /api/v1/courses/course-basics/modules/module-intro/lessons/lesson-welcome",
			"PUT /api/v1/courses/course-basics/lessons/lesson-welcome/challenges/challenge-choice",
			"GET /api/v1/courses/course-basics/structure?language=en",
			"GET /api/v1/courses/course-basics/modules/module-intro/lessons/lesson-welcome",
			"GET /api/v1/courses/course-basics/assets",
			"GET /api/v1/courses/course-basics/app-links",
			"PUT /api/v1/courses/course-basics",
		]);

		const courseBody = {
			language: "en",
			slug: "flow-like-basics",
			difficulty: "INTERMEDIATE",
			category: "FLOWS",
			estimated_minutes: 25,
			is_published: false,
			icon_url: null,
			banner_url: null,
			tags: ["basics", "agents"],
			position: 2,
			name: "Flow-Like basics",
			description: "Build your first Flow-Like workflow.",
			long_description: "A complete, agent-authored introduction.",
		};
		expect(requests[0]?.body).toEqual(courseBody);
		expect(requests[1]?.body).toEqual({
			title: "Introduction",
			description: "Start here.",
			position: 3,
		});
		expect(requests[2]?.body).toEqual({
			title: "Welcome",
			language: "en",
			content: "# Welcome",
			video_url: "https://video.example/welcome",
			estimated_minutes: 7,
			position: 4,
			is_optional: false,
		});
		expect(requests[3]?.body).toEqual({
			kind: "SINGLE_CHOICE",
			prompt: "Which answer is correct?",
			explanation: "The first answer is correct.",
			payload: remoteChallenge.payload,
			points: 15,
			position: 6,
		});
		expect(requests.at(-1)?.body).toEqual({
			...courseBody,
			is_published: true,
		});
	});

	test("stops on the first failure, skips remaining work, and redacts secrets", async () => {
		globalThis.fetch = vi.fn(
			async () =>
				new Response(
					JSON.stringify({
						error: {
							code: "INTERNAL_ERROR",
							message:
								"failed for pat_test.secret at https://blob.example/file?X-Amz-Credential=upload-user&X-Amz-Signature=upload-secret",
						},
					}),
					{ status: 500, headers: { "Content-Type": "application/json" } },
				),
		) as unknown as typeof fetch;

		const result = await runUniversityPlan(authoredPlan(true), remoteOptions);
		const serialized = JSON.stringify(result);

		expect(result).toMatchObject({
			command: "apply",
			passed: false,
			courseId: "course-basics",
		});
		expect(result.operations?.map(({ status }) => status)).toEqual([
			"failed",
			"skipped",
			"skipped",
			"skipped",
			"skipped",
		]);
		expect(result.error).toContain("failed for [REDACTED]");
		expect(result.error).toContain("X-Amz-Credential=[REDACTED]");
		expect(result.error).toContain("X-Amz-Signature=[REDACTED]");
		expect(serialized).not.toContain("pat_test.secret");
		expect(serialized).not.toContain("upload-secret");
	});

	test("sanitizes URLs and removes signed URLs and attempts from list output", async () => {
		globalThis.fetch = vi.fn(
			async () =>
				new Response(
					JSON.stringify([
						{
							id: "course-basics",
							icon_url:
								"https://cdn-user:cdn-pass@cdn.example/icon.png?signature=secret#fragment",
							signed_url: "https://blob.example/file?sig=secret",
							attempts: [{ submission: "private" }],
						},
					]),
					{ status: 200, headers: { "Content-Type": "application/json" } },
				),
		) as unknown as typeof fetch;

		const result = await listUniversityCourses("en", remoteOptions);
		const data = result.data as Array<Record<string, unknown>>;

		expect(result, result.error).toMatchObject({ passed: true });
		expect(data).toEqual([
			{
				id: "course-basics",
				icon_url: "https://cdn.example/icon.png",
			},
		]);
		expect(JSON.stringify(result)).not.toContain("cdn-pass");
		expect(JSON.stringify(result)).not.toContain("secret");
		expect(JSON.stringify(result)).not.toContain("private");
	});

	test("sanitizes presigned URLs embedded inside lesson content", async () => {
		globalThis.fetch = vi.fn(async (input) => {
			const url = String(input);
			if (url.endsWith("/structure")) {
				return jsonResponse({
					course: { id: "course-basics" },
					modules: [
						{
							id: "module-basics",
							lessons: [{ id: "lesson-basics" }],
						},
					],
				});
			}
			if (url.includes("/modules/module-basics/lessons/lesson-basics")) {
				return jsonResponse({
					lesson: {
						id: "lesson-basics",
						content:
							'{"url":"https://storage.example/file.webp?X-Amz-Credential=user&X-Amz-Signature=secret"}',
					},
					challenges: [],
					app_refs: [],
				});
			}
			if (url.endsWith("/assets") || url.endsWith("/app-links")) {
				return jsonResponse([]);
			}
			throw new Error(`Unexpected request: ${url}`);
		}) as unknown as typeof fetch;

		const result = await inspectUniversityCourse(
			"course-basics",
			undefined,
			remoteOptions,
		);
		const serialized = JSON.stringify(result);

		expect(result, result.error).toMatchObject({ passed: true });
		expect(serialized).toContain("https://storage.example/file.webp");
		expect(serialized).not.toContain("X-Amz-Credential");
		expect(serialized).not.toContain("X-Amz-Signature");
		expect(serialized).not.toContain("secret");
	});

	test.skipIf(!canStubGlobals)(
		"explicitly replaces a same-named asset even when metadata matches",
		async () => {
			stubBunFile();
			const directory = await mkdtemp(
				join(tmpdir(), "flow-like-university-runner-"),
			);
			temporaryDirectories.push(directory);
			const file = join(directory, "overview.png");
			await writeFile(file, new Uint8Array([1, 2, 3, 4]));
			const existing = {
				id: "asset-old",
				course_id: "course-basics",
				name: "EditorOverview",
				filename: "overview.png",
				mime_type: "image/png",
				size: 4,
				kind: "IMAGE",
				created_at: "2026-08-13T00:00:00Z",
				updated_at: "2026-08-13T00:00:00Z",
			};
			const replacement = { ...existing, id: "asset-new" };
			const calls: Array<{ method: string; url: string }> = [];
			globalThis.fetch = vi.fn(async (input, init) => {
				const method = init?.method ?? "GET";
				const url = String(input);
				calls.push({ method, url });
				if (method === "GET") {
					return new Response(JSON.stringify([existing]), { status: 200 });
				}
				if (method === "DELETE") return new Response(null, { status: 204 });
				if (method === "POST") {
					return new Response(
						JSON.stringify({
							asset: replacement,
							signed_url: "https://storage.example/upload?sig=secret",
						}),
						{ status: 200 },
					);
				}
				return new Response(null, { status: 204 });
			}) as unknown as typeof fetch;

			const result = await uploadSingleUniversityAsset(
				{
					courseId: "course-basics",
					name: "EditorOverview",
					file,
					replace: true,
				},
				remoteOptions,
			);

			expect(result, result.error).toMatchObject({ passed: true });
			expect(result.operations?.[0]?.status).toBe("completed");
			expect(calls.map(({ method }) => method)).toEqual([
				"GET",
				"DELETE",
				"POST",
				"PUT",
			]);
		},
	);

	test.skipIf(!canStubGlobals)(
		"fails verification when an asset record exists but its file is missing",
		async () => {
			stubBunFile();
			const directory = await mkdtemp(
				join(tmpdir(), "flow-like-university-runner-"),
			);
			temporaryDirectories.push(directory);
			const file = join(directory, "overview.webp");
			await writeFile(file, new Uint8Array([1, 2, 3, 4]));

			const plan = validateUniversityPlan({
				schema: UNIVERSITY_PLAN_SCHEMA,
				course: {
					id: "course-basics",
					name: "Flow-Like basics",
					assets: [
						{
							name: "EditorOverview",
							file,
							kind: "IMAGE",
							mimeType: "image/webp",
							filename: "overview.webp",
							replace: true,
						},
					],
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
							],
						},
					],
				},
			});
			(plan.course.assets[0] as { size: number }).size = 4;

			const remoteCourse = {
				id: "course-basics",
				language: "en",
				slug: null,
				difficulty: "BEGINNER",
				category: "GENERAL",
				estimated_minutes: 0,
				is_published: false,
				icon_url: null,
				banner_url: null,
				tags: [],
				position: null,
				name: "Flow-Like basics",
				description: null,
				long_description: null,
			};
			const remoteAsset = {
				id: "asset-overview",
				course_id: "course-basics",
				name: "EditorOverview",
				filename: "overview.webp",
				mime_type: "image/webp",
				size: 4,
				kind: "IMAGE",
				created_at: "2026-08-13T00:00:00Z",
				updated_at: "2026-08-13T00:00:00Z",
			};
			let assetCreated = false;
			globalThis.fetch = vi.fn(async (input, init) => {
				const method = init?.method ?? "GET";
				const url = new URL(String(input));
				if (url.hostname === "storage.example") {
					if (method === "PUT") return new Response(null, { status: 204 });
					return new Response("missing", {
						status: 404,
						headers: { "Content-Type": "application/xml" },
					});
				}
				if (
					method === "PUT" &&
					url.pathname === "/api/v1/courses/course-basics"
				) {
					return jsonResponse(remoteCourse);
				}
				if (method === "POST" && url.pathname.endsWith("/assets")) {
					assetCreated = true;
					return jsonResponse({
						asset: remoteAsset,
						signed_url: "https://storage.example/upload?sig=secret",
					});
				}
				if (method === "GET" && url.pathname.endsWith("/assets")) {
					return jsonResponse(assetCreated ? [remoteAsset] : []);
				}
				if (method === "GET" && url.pathname.endsWith("/structure")) {
					return jsonResponse({
						course: remoteCourse,
						modules: [
							{
								id: "module-intro",
								course_id: "course-basics",
								title: "Introduction",
								description: null,
								position: 0,
								lessons: [{ id: "lesson-welcome" }],
							},
						],
					});
				}
				if (method === "GET" && url.pathname.endsWith("/lesson-welcome")) {
					return jsonResponse({
						lesson: {
							id: "lesson-welcome",
							module_id: "module-intro",
							title: "Welcome",
							language: "en",
							content: "# Welcome",
							video_url: null,
							estimated_minutes: 5,
							position: 0,
							is_optional: false,
						},
						challenges: [],
						app_refs: [],
						attempts: [],
						assets: [
							{
								...remoteAsset,
								signed_url: "https://storage.example/download?sig=secret",
							},
						],
					});
				}
				if (method === "GET" && url.pathname.endsWith("/app-links")) {
					return jsonResponse([]);
				}
				return jsonResponse({ ok: true });
			}) as unknown as typeof fetch;

			const result = await runUniversityPlan(plan, remoteOptions);

			expect(result).toMatchObject({ command: "apply", passed: false });
			expect(result.error).toContain(
				"asset @EditorOverview could not be downloaded (HTTP 404)",
			);
			expect(JSON.stringify(result)).not.toContain("sig=secret");
		},
	);
});
