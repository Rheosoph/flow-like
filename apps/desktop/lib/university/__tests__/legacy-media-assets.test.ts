import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test, vi } from "vitest";

import { loadUniversityPlan } from "../plan";
import { runUniversityPlan, uploadSingleUniversityAsset } from "../runner";
import { UNIVERSITY_PLAN_SCHEMA } from "../types";

const originalFetch = globalThis.fetch;
const directories: string[] = [];
const fixture = new TextEncoder().encode('{"message":"fixture data"}\n');
const remoteOptions = {
	apiUrl: "https://flow.example",
	pat: "pat_test.secret",
	timeoutMs: 5_000,
};

afterEach(async () => {
	globalThis.fetch = originalFetch;
	vi.restoreAllMocks();
	await Promise.all(
		directories
			.splice(0)
			.map((path) => rm(path, { recursive: true, force: true })),
	);
});

async function practiceFile(filename = "cases.json") {
	const directory = await mkdtemp(join(tmpdir(), "university-legacy-media-"));
	directories.push(directory);
	const file = join(directory, filename);
	await writeFile(file, fixture);
	return { directory, file };
}

function json(value: unknown) {
	return new Response(JSON.stringify(value), {
		headers: { "Content-Type": "application/json" },
	});
}

function remote(existing: Record<string, unknown>[] = []) {
	const created: Record<string, unknown>[] = [];
	const uploaded: Array<{ bytes: Uint8Array; headers: Headers }> = [];
	const deleted: string[] = [];
	let assets = [...existing];
	let course: Record<string, unknown> = {};
	let module: Record<string, unknown> = {};
	let lesson: Record<string, unknown> = {};
	globalThis.fetch = vi.fn(async (input, init) => {
		const url = new URL(String(input));
		const method = init?.method ?? "GET";
		if (url.hostname === "storage.example") {
			if (method === "PUT") {
				uploaded.push({
					bytes: new Uint8Array(await new Response(init?.body).arrayBuffer()),
					headers: new Headers(init?.headers),
				});
				return new Response(null, { status: 204 });
			}
			return new Response(fixture, {
				status: 206,
				headers: { "Content-Type": String(assets[0]?.mime_type) },
			});
		}
		const body = init?.body ? JSON.parse(String(init.body)) : {};
		if (url.pathname.endsWith("/assets")) {
			if (method === "GET") return json(assets);
			if (method === "POST") {
				created.push(body);
				const asset = {
					...body,
					id: "asset-new",
					course_id: "course-practice",
				};
				assets.push(asset);
				return json({
					asset,
					signed_url: `https://storage.example/asset-new.${body.extension}`,
				});
			}
		}
		if (method === "DELETE" && url.pathname.includes("/assets/")) {
			const assetId = url.pathname.split("/").at(-1) ?? "";
			deleted.push(assetId);
			assets = assets.filter((asset) => asset.id !== assetId);
			return json(null);
		}
		if (url.pathname.endsWith("/app-links")) return json([]);
		if (url.pathname.endsWith("/structure")) {
			return json({ course, modules: [{ ...module, lessons: [lesson] }] });
		}
		if (url.pathname.includes("/lessons/")) {
			if (method === "PUT") {
				lesson = {
					...body,
					id: "lesson-practice",
					module_id: "module-practice",
				};
				return json(lesson);
			}
			return json({
				lesson,
				challenges: [],
				app_refs: [],
				assets: assets.map((asset) => ({
					...asset,
					signed_url: `https://storage.example/${asset.id}.${asset.extension}`,
				})),
			});
		}
		if (url.pathname.includes("/modules/") && method === "PUT") {
			module = { ...body, id: "module-practice", course_id: "course-practice" };
			return json(module);
		}
		if (url.pathname.endsWith("/courses/course-practice") && method === "PUT") {
			course = { ...body, id: "course-practice" };
			return json(course);
		}
		throw new Error(`Unexpected mock request: ${method} ${url.pathname}`);
	}) as unknown as typeof fetch;
	return { created, uploaded, deleted };
}

describe("University legacy media compatibility", () => {
	test("keeps the original storage extension by default", async () => {
		const { file } = await practiceFile();
		const mock = remote();
		const result = await uploadSingleUniversityAsset(
			{
				courseId: "course-practice",
				name: "PracticeFiles",
				file,
				replace: false,
			},
			remoteOptions,
		);
		expect(result.passed, result.error).toBe(true);
		expect(mock.created[0]).toMatchObject({
			extension: "json",
			filename: "cases.json",
			mime_type: "application/json",
			kind: "DOCUMENT",
		});
		expect(mock.uploaded[0].bytes).toEqual(fixture);
	});

	test("uses an opaque key while preserving document bytes and download metadata", async () => {
		const { file } = await practiceFile("résumé cases.json");
		const mock = remote();
		const result = await uploadSingleUniversityAsset(
			{
				courseId: "course-practice",
				name: "PracticeFiles",
				file,
				replace: false,
				legacyMediaAssets: true,
			},
			remoteOptions,
		);
		expect(result.passed, result.error).toBe(true);
		expect(mock.created[0]).toMatchObject({
			extension: "webp",
			filename: "résumé cases.json",
			mime_type: "application/json",
			kind: "DOCUMENT",
			size: fixture.length,
		});
		expect(mock.uploaded[0].bytes).toEqual(fixture);
		expect(mock.uploaded[0].headers.get("Content-Type")).toBe(
			"application/json",
		);
		const disposition = mock.uploaded[0].headers.get("Content-Disposition");
		expect(disposition).toContain("attachment;");
		expect(disposition).toContain(
			"filename*=UTF-8''r%C3%A9sum%C3%A9%20cases.json",
		);
	});

	test("applies the same document behavior when importing a course plan", async () => {
		const { directory, file } = await practiceFile();
		const planFile = join(directory, "course.plan.json");
		await writeFile(
			planFile,
			JSON.stringify({
				schema: UNIVERSITY_PLAN_SCHEMA,
				course: {
					id: "course-practice",
					name: "Practice course",
					isPublished: false,
					assets: [{ name: "PracticeFiles", file, kind: "DOCUMENT" }],
					modules: [
						{
							id: "module-practice",
							title: "Practice",
							lessons: [
								{
									id: "lesson-practice",
									title: "Practice",
									content: "@PracticeFiles",
								},
							],
						},
					],
				},
			}),
		);
		const plan = await loadUniversityPlan(planFile);
		const mock = remote();
		const result = await runUniversityPlan(plan, remoteOptions, {
			legacyMediaAssets: true,
		});
		expect(result.passed, result.error).toBe(true);
		expect(mock.created[0]).toMatchObject({
			extension: "webp",
			filename: "cases.json",
			mime_type: "application/json",
			kind: "DOCUMENT",
		});
		expect(mock.uploaded[0].bytes).toEqual(fixture);
		expect(mock.uploaded[0].headers.get("Content-Disposition")).toContain(
			'filename="cases.json"',
		);
		expect(plan.course.assets[0].extension).toBe("json");
	});

	test("does not change image object extensions", async () => {
		const { file } = await practiceFile("diagram.png");
		const mock = remote();
		const result = await uploadSingleUniversityAsset(
			{
				courseId: "course-practice",
				name: "Diagram",
				file,
				replace: false,
				legacyMediaAssets: true,
			},
			remoteOptions,
		);
		expect(result.passed, result.error).toBe(true);
		expect(mock.created[0]).toMatchObject({
			extension: "png",
			filename: "diagram.png",
			mime_type: "image/png",
			kind: "IMAGE",
		});
		expect(mock.uploaded[0].bytes).toEqual(fixture);
	});

	test("requires explicit replacement instead of skipping an existing document record", async () => {
		const { file } = await practiceFile();
		const mock = remote([
			{
				id: "asset-old",
				course_id: "course-practice",
				name: "PracticeFiles",
				filename: "cases.json",
				mime_type: "application/json",
				kind: "DOCUMENT",
				size: fixture.length,
			},
		]);
		const result = await uploadSingleUniversityAsset(
			{
				courseId: "course-practice",
				name: "PracticeFiles",
				file,
				replace: false,
				legacyMediaAssets: true,
			},
			remoteOptions,
		);
		expect(result.passed).toBe(false);
		expect(result.error).toMatch(/replace/i);
		expect(mock.created).toHaveLength(0);
		expect(mock.uploaded).toHaveLength(0);
		expect(mock.deleted).toHaveLength(0);
	});
});
