import { afterEach, describe, expect, test } from "vitest";
import { validateUniversityPlan } from "../plan";
import { planUniversityRun, runUniversityPlan } from "../runner";
import { UNIVERSITY_PLAN_SCHEMA } from "../types";

const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

const remoteOptions = {
	apiUrl: "https://flow.example",
	pat: "pat_test.secret",
	timeoutMs: 5_000,
};

function plan() {
	return validateUniversityPlan({
		schema: UNIVERSITY_PLAN_SCHEMA,
		course: {
			id: "course",
			name: "Focused course",
			isPublished: true,
			modules: [
				{
					id: "keep-module",
					title: "Module",
					lessons: [
						{
							id: "keep-lesson",
							title: "Lesson",
							content: "Practice one action.",
							challenges: [
								{
									id: "keep-challenge",
									kind: "SINGLE_CHOICE",
									prompt: "Choose the result.",
									payload: {
										options: [
											{ id: "a", label: "Correct" },
											{ id: "b", label: "Incorrect" },
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
}

type Row = Record<string, unknown>;
function json(value: unknown, status = 200) {
	return new Response(JSON.stringify(value), {
		status,
		headers: { "Content-Type": "application/json" },
	});
}

function server(
	options: {
		deletion?: "queued" | "ineffective" | "failed";
		retainedDescendant?: boolean;
		wrongCourse?: boolean;
		omitTarget?: boolean;
		descendantReadFailure?: boolean;
	} = {},
) {
	let course: Row = {};
	const modules = new Map<string, Row>([
		["keep-module", { id: "keep-module", course_id: "course" }],
		[
			"old-module",
			{
				id: "old-module",
				course_id: options.wrongCourse ? "another-course" : "course",
			},
		],
	]);
	const lessons = new Map<string, Row>([
		["keep-lesson", { id: "keep-lesson", module_id: "keep-module" }],
		["old-lesson", { id: "old-lesson", module_id: "keep-module" }],
		["old-nested-lesson", { id: "old-nested-lesson", module_id: "old-module" }],
	]);
	const challenges = new Map<string, Row>(
		Array.from({ length: 10 }, (_, index) => [
			`old-challenge-${index}`,
			{ id: `old-challenge-${index}`, lesson_id: "keep-lesson" },
		]),
	);
	const refs = new Map<string, Row>([
		["old-ref", { id: "old-ref", lesson_id: "keep-lesson" }],
	]);
	const links = new Map<string, Row>([
		["old-link", { id: "old-link", course_id: "course" }],
	]);
	const calls: Array<{ method: string; path: string; published?: unknown }> =
		[];
	globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
		const url = new URL(String(input));
		const parts = url.pathname
			.replace(/^\/api\/v1/, "")
			.split("/")
			.filter(Boolean);
		const method = init?.method ?? "GET";
		const body: Row = init?.body ? JSON.parse(String(init.body)) : {};
		calls.push({ method, path: url.pathname, published: body.is_published });
		if (method === "PUT") {
			if (parts.length === 2) {
				course = { id: "course", ...body };
				return json(course);
			}
			if (parts[2] === "modules" && parts.length === 4) {
				modules.set(parts[3], { id: parts[3], course_id: "course", ...body });
			} else if (parts[2] === "modules" && parts.length === 6) {
				if (options.omitTarget) lessons.delete(parts[5]);
				else
					lessons.set(parts[5], { id: parts[5], module_id: parts[3], ...body });
			} else if (parts[4] === "challenges") {
				challenges.set(parts[5], {
					id: parts[5],
					lesson_id: parts[3],
					...body,
				});
			} else throw new Error(`Unexpected PUT ${url.pathname}`);
			return json({ ok: true });
		}
		if (method === "DELETE") {
			if (options.deletion === "failed")
				return json({ message: "Deletion failed" }, 500);
			if (options.deletion !== "ineffective") {
				if (parts[2] === "modules" && parts.length === 4) {
					modules.delete(parts[3]);
					for (const [lessonId, lesson] of lessons)
						if (lesson.module_id === parts[3]) lessons.delete(lessonId);
				} else if (parts[2] === "modules") lessons.delete(parts[5]);
				else if (parts[4] === "challenges") challenges.delete(parts[5]);
				else if (parts[4] === "refs") refs.delete(parts[5]);
				else if (parts[2] === "app-links") links.delete(parts[3]);
				else throw new Error(`Unexpected DELETE ${url.pathname}`);
			}
			return options.deletion === "queued"
				? json({ job_id: "deletion-job", status: "QUEUED" }, 202)
				: json(null);
		}
		if (method === "GET") {
			if (parts[2] === "structure")
				return json({
					course,
					modules: [...modules.values()].map((module) => ({
						...module,
						lessons: [...lessons.values()]
							.filter((lesson) => lesson.module_id === module.id)
							.map((lesson) => ({ id: lesson.id })),
					})),
				});
			if (parts[2] === "app-links") return json([...links.values()]);
			if (parts[2] === "assets") return json([]);
			if (parts[2] === "modules" && parts.length === 6) {
				const lessonId = parts[5];
				if (options.descendantReadFailure && lessonId === "old-nested-lesson")
					return json({ message: "Descendant read failed" }, 500);
				const nested = [...challenges.values()].filter(
					(challenge) => challenge.lesson_id === lessonId,
				);
				if (options.retainedDescendant && lessonId === "old-lesson")
					nested.push({ id: "keep-challenge", lesson_id: lessonId });
				return json({
					lesson: lessons.get(lessonId),
					challenges: nested,
					app_refs: [...refs.values()].filter(
						(ref) => ref.lesson_id === lessonId,
					),
					assets: [],
					attempts: [],
				});
			}
		}
		throw new Error(`Unexpected ${method} ${url.pathname}`);
	}) as typeof fetch;
	return {
		calls,
		modules,
		lessons,
		challenges,
		refs,
		links,
		published: () => course.is_published,
	};
}

describe("University course reconciliation", () => {
	test("requires explicit pruning and reports every retirement without publishing", async () => {
		const remote = server();
		const result = await runUniversityPlan(plan(), remoteOptions);
		expect(result.passed).toBe(false);
		expect(result.error).toContain("old-challenge-9");
		expect(result.error).toContain("old-link");
		expect(result.error).toContain("--prune");
		expect(
			(result.data as { retirements: unknown[] }).retirements,
		).toHaveLength(14);
		expect(remote.calls.filter((call) => call.method === "DELETE")).toEqual([]);
		expect(remote.published()).toBe(false);
	});

	test("upserts targets, prunes only obsolete roots, verifies, then publishes", async () => {
		const remote = server();
		const result = await runUniversityPlan(plan(), remoteOptions, {
			prune: true,
		});
		expect(result.error).toBeUndefined();
		expect(result.passed).toBe(true);
		const deletes = remote.calls.filter((call) => call.method === "DELETE");
		expect(deletes).toHaveLength(14);
		expect(
			deletes.some((call) => call.path.includes("old-nested-lesson")),
		).toBe(false);
		expect(
			deletes.some(
				(call) =>
					call.path.includes("keep-challenge") || call.path.includes("assets"),
			),
		).toBe(false);
		expect(remote.challenges.has("keep-challenge")).toBe(true);
		expect(remote.lessons.has("keep-lesson")).toBe(true);
		expect(
			remote.calls.findIndex((call) => call.method === "DELETE"),
		).toBeGreaterThan(
			remote.calls.findIndex(
				(call) => call.method === "PUT" && call.path.endsWith("keep-challenge"),
			),
		);
		expect(remote.calls.at(-1)).toMatchObject({
			method: "PUT",
			published: true,
		});
		expect(remote.published()).toBe(true);
		expect(
			(
				result.data as { retirements: Array<{ status: string }> }
			).retirements.every((item) => item.status === "completed"),
		).toBe(true);
	});

	test("keeps prune dry runs offline and before verification", () => {
		globalThis.fetch = (() => {
			throw new Error("No network allowed");
		}) as unknown as typeof fetch;
		const result = planUniversityRun(plan(), { prune: true });
		expect(result.passed).toBe(true);
		expect(
			result.operations?.slice(-3).map((operation) => operation.type),
		).toEqual(["prune", "verify", "upsertCourse"]);
	});

	test.each([
		[{ wrongCourse: true }, "unexpected parent"],
		[{ retainedDescendant: true }, "explicit parent migration"],
		[{ omitTarget: true }, "missing after upsert"],
		[{ descendantReadFailure: true }, "Descendant read failed"],
	] as const)(
		"inspects all descendants before deleting any root: %j",
		async (options, message) => {
			const remote = server(options);
			const result = await runUniversityPlan(plan(), remoteOptions, {
				prune: true,
			});
			expect(result.passed).toBe(false);
			expect(result.error).toContain(message);
			expect(remote.calls.some((call) => call.method === "DELETE")).toBe(false);
			expect(remote.published()).toBe(false);
		},
	);

	test.each([
		["queued", "deletion-job"],
		["ineffective", "unexpected challenge"],
		["failed", "Deletion failed"],
	] as const)(
		"never publishes after %s deletion",
		async (deletion, message) => {
			const remote = server({ deletion });
			const result = await runUniversityPlan(plan(), remoteOptions, {
				prune: true,
			});
			expect(result.passed).toBe(false);
			expect(result.error).toContain(message);
			expect(remote.published()).toBe(false);
			expect(remote.calls.some((call) => call.published === true)).toBe(false);
		},
	);
});
