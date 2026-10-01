import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { LearningPathUpsertBody } from "../api-types";
import { applyLearningPaths } from "../learning-paths";

const directories: string[] = [];
const originalFetch = globalThis.fetch;
const options = {
	apiUrl: "https://flow.example",
	pat: "pat_example",
	timeoutMs: 5000,
};

afterEach(async () => {
	globalThis.fetch = originalFetch;
	vi.restoreAllMocks();
	await Promise.all(
		directories
			.splice(0)
			.map((directory) => rm(directory, { recursive: true, force: true })),
	);
});

async function fixture(invalid = false) {
	const directory = await mkdtemp(join(tmpdir(), "university-paths-"));
	directories.push(directory);
	for (const id of ["first", "second", "elective"]) {
		const path = join(directory, "courses", "core", id);
		await mkdir(path, { recursive: true });
		await writeFile(
			join(path, "course.plan.json"),
			JSON.stringify({
				schema: "flow-like.university-plan/v1",
				course: {
					id,
					name: id,
					estimatedMinutes: invalid ? 99 : 5,
					modules: [
						{
							id: `${id}-module`,
							title: "Practice",
							lessons: [
								{
									id: `${id}-lesson`,
									title: "Run",
									content: "Run the practice flow.",
									estimatedMinutes: 5,
								},
							],
						},
					],
				},
			}),
		);
	}
	const path = join(directory, "curriculum.json");
	await writeFile(
		path,
		JSON.stringify({
			schema: "flow-like.university-curriculum/v1",
			paths: [
				{
					id: "path-first",
					slug: "first",
					name: "First workflow",
					description: "Build a flow.",
					courses: [
						"courses/core/first/course.plan.json",
						"courses/core/second/course.plan.json",
					],
					electives: ["courses/core/elective/course.plan.json"],
				},
			],
		}),
	);
	return path;
}

function api(
	config: {
		extra?: boolean;
		omitStep?: boolean;
		unpublished?: boolean;
		losePublishResponse?: boolean;
		corruptPublished?: boolean;
		recoveryFails?: boolean;
	} = {},
) {
	const calls: Array<{
		method: string;
		path: string;
		body: Partial<LearningPathUpsertBody>;
		headers: Headers;
	}> = [];
	let metadata: Partial<LearningPathUpsertBody> = {};
	let publicationAttempted = false;
	const steps = new Map<string, number>(
		config.extra ? [["old-course", 9]] : [],
	);
	const view = () => ({
		id: "path-first",
		...metadata,
		steps: [...steps].map(([course_id, position]) => ({
			course_id,
			position,
			course: { id: course_id, is_published: !config.unpublished },
		})),
	});
	globalThis.fetch = vi.fn(
		async (input: RequestInfo | URL, init?: RequestInit) => {
			const url = new URL(String(input));
			const body: Partial<LearningPathUpsertBody> = init?.body
				? JSON.parse(String(init.body))
				: {};
			calls.push({
				method: init?.method ?? "GET",
				path: url.pathname,
				body,
				headers: new Headers(init?.headers),
			});
			const step = url.pathname.match(/\/courses\/([^/]+)$/);
			if (url.pathname === "/api/v1/courses/paths/path-first") {
				if (init?.method === "PUT") {
					if (
						publicationAttempted &&
						!body.is_published &&
						config.recoveryFails
					)
						return new Response("recovery failed pat_example", { status: 503 });
					metadata = body;
					if (body.is_published) {
						publicationAttempted = true;
						if (config.losePublishResponse)
							throw new Error("response lost pat_example");
						if (config.corruptPublished)
							return Response.json({ ...view(), title: "unexpected" });
					}
				}
				return Response.json(view());
			}
			if (step && init?.method === "PUT") {
				if (!config.omitStep || step[1] !== "second")
					steps.set(step[1], body.position ?? 0);
				return Response.json(null);
			}
			throw new Error(`Unexpected request ${init?.method} ${url.pathname}`);
		},
	) as unknown as typeof fetch;
	return { calls, view };
}

describe("learning path publication", () => {
	test("stages core steps, verifies readback, then publishes with electives in the description", async () => {
		const path = await fixture();
		const fake = api();
		const result = await applyLearningPaths(path, options);
		expect(result.passed).toBe(true);
		expect(result.paths).toEqual([{ id: "path-first", status: "published" }]);
		expect(fake.calls.map((call) => [call.method, call.path])).toEqual([
			["PUT", "/api/v1/courses/paths/path-first"],
			["PUT", "/api/v1/courses/paths/path-first/courses/first"],
			["PUT", "/api/v1/courses/paths/path-first/courses/second"],
			["GET", "/api/v1/courses/paths/path-first"],
			["PUT", "/api/v1/courses/paths/path-first"],
		]);
		expect(fake.calls[0].body.is_published).toBe(false);
		expect(fake.calls[4].body.is_published).toBe(true);
		expect(fake.calls[0].body.description).toContain(
			"Optional courses: elective.",
		);
		expect(
			fake.calls.every(
				(call) => call.headers.get("Authorization") === options.pat,
			),
		).toBe(true);
		expect(fake.view().steps.map((step) => step.course_id)).toEqual([
			"first",
			"second",
		]);
	});

	test("leaves unexpected remote steps untouched and the path unpublished", async () => {
		const path = await fixture();
		const fake = api({ extra: true });
		const result = await applyLearningPaths(path, options);
		expect(result.passed).toBe(false);
		expect(result.paths[0].status).toBe("draft");
		expect(result.errors[0]).toContain("Unexpected remote steps: old-course");
		expect(fake.calls).toHaveLength(1);
		expect(fake.view().is_published).toBe(false);
		expect(fake.view().steps[0].course_id).toBe("old-course");
	});

	test.each([{ omitStep: true }, { unpublished: true }])(
		"refuses publication when readback cannot verify usable core steps: %j",
		async (config) => {
			const path = await fixture();
			const fake = api(config);
			const result = await applyLearningPaths(path, options);
			expect(result.passed).toBe(false);
			expect(result.paths[0].status).toBe("draft");
			expect(fake.calls.some((call) => call.body?.is_published === true)).toBe(
				false,
			);
		},
	);

	test("restores a draft when the publication response is lost", async () => {
		const path = await fixture();
		const fake = api({ losePublishResponse: true });
		const result = await applyLearningPaths(path, options);
		expect(result.paths[0].status).toBe("draft");
		expect(fake.view().is_published).toBe(false);
		expect(JSON.stringify(result)).not.toContain("pat_example");
		expect(fake.calls.at(-1)?.body.is_published).toBe(false);
	});

	test("reports uncertain state if recovery after a bad publication response also fails", async () => {
		const path = await fixture();
		api({ corruptPublished: true, recoveryFails: true });
		const result = await applyLearningPaths(path, options);
		expect(result.paths[0].status).toBe("failed");
		expect(result.errors[0]).toContain("Could not confirm draft recovery");
		expect(JSON.stringify(result)).not.toContain("pat_example");
	});

	test("never calls the API when local curriculum validation fails", async () => {
		const path = await fixture(true);
		const fake = api();
		const result = await applyLearningPaths(path, options);
		expect(result.passed).toBe(false);
		expect(fake.calls).toHaveLength(0);
	});

	test("reuses client restrictions on insecure hosts and invalid credentials", async () => {
		const path = await fixture();
		const fake = api();
		await expect(
			applyLearningPaths(path, { ...options, apiUrl: "http://remote.example" }),
		).rejects.toThrow("https");
		await expect(
			applyLearningPaths(path, { ...options, pat: "api_key" }),
		).rejects.toThrow("PAT");
		expect(fake.calls).toHaveLength(0);
	});
});
