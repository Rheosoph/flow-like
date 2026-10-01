import { readFile } from "node:fs/promises";
import type { LearningPathUpsertBody, LearningPathView } from "./api-types";
import { createUniversityClient } from "./client";
import {
	type CurriculumCheckResult,
	checkCurriculum,
	validateCurriculum,
} from "./curriculum";
import type { UniversityRemoteOptions } from "./runner";

export interface LearningPathsResult {
	schema: "flow-like.university-paths-result/v1";
	passed: boolean;
	check: CurriculumCheckResult;
	paths: Array<{
		id: string;
		status: "published" | "draft" | "failed";
		error?: string;
	}>;
	errors: string[];
}

function errorMessage(error: unknown): string {
	return (error instanceof Error ? error.message : String(error))
		.replace(/pat_[A-Za-z0-9._-]+/g, "[REDACTED]")
		.replace(
			/([?&][^=&\s]*(?:token|key|secret|password|auth|signature|credential)[^=&\s]*=)[^&\s]+/gi,
			"$1[REDACTED]",
		);
}

function verifyMetadata(
	view: LearningPathView,
	id: string,
	body: LearningPathUpsertBody,
): void {
	if (
		view.id !== id ||
		view.title !== body.title ||
		view.slug !== body.slug ||
		view.description !== body.description ||
		view.position !== body.position ||
		view.is_published !== body.is_published ||
		!Array.isArray(view.steps)
	)
		throw new Error(
			`Path ${id} did not return the requested metadata and ${body.is_published ? "published" : "draft"} state.`,
		);
}

function unexpectedSteps(view: LearningPathView, expected: string[]): string[] {
	return view.steps
		.filter((step) => !expected.includes(step.course_id))
		.map((step) => step.course_id);
}

function verifySteps(view: LearningPathView, expected: string[]): void {
	const extra = unexpectedSteps(view, expected);
	if (extra.length)
		throw new Error(
			`Unexpected remote steps: ${extra.join(", ")}. Remove them deliberately in the authoring interface, then retry.`,
		);
	if (view.steps.length !== expected.length)
		throw new Error(
			"Remote path step count does not match the core course list.",
		);
	for (const [position, id] of expected.entries()) {
		const step = view.steps.find((item) => item.position === position);
		if (!step || step.course_id !== id)
			throw new Error(`Expected core course ${id} at position ${position}.`);
		if (
			!step.course ||
			step.course.id !== id ||
			step.course.is_published !== true
		)
			throw new Error(
				`Core course ${id} is missing or unpublished remotely; apply and publish its course plan first.`,
			);
	}
}

/** Applies only ordered core steps. No course, lesson, asset, or step is deleted. */
export async function applyLearningPaths(
	catalogPath: string,
	options: UniversityRemoteOptions,
): Promise<LearningPathsResult> {
	const check = await checkCurriculum(catalogPath);
	const result: LearningPathsResult = {
		schema: "flow-like.university-paths-result/v1",
		passed: false,
		check,
		paths: [],
		errors: [...check.errors],
	};
	if (!check.passed) return result;
	if (
		!Number.isSafeInteger(options.timeoutMs) ||
		options.timeoutMs < 1 ||
		options.timeoutMs > 300_000
	)
		throw new Error(
			"Learning path timeout must be an integer from 1 to 300000 ms.",
		);
	const curriculum = validateCurriculum(
		JSON.parse(await readFile(catalogPath, "utf8")),
	);
	const byPlan = new Map(check.courses.map((course) => [course.plan, course]));
	for (const path of curriculum.paths)
		for (const reference of [...path.courses, ...path.electives])
			if (!byPlan.has(reference))
				throw new Error(
					"Curriculum changed during validation; rerun before applying paths.",
				);
	const courseFor = (reference: string) => {
		const course = byPlan.get(reference);
		if (!course) throw new Error(`Missing validated course: ${reference}`);
		return course;
	};
	const controller = new AbortController();
	const client = createUniversityClient({
		baseUrl: options.apiUrl,
		pat: options.pat,
		signal: controller.signal,
	});
	const timeout = setTimeout(() => controller.abort(), options.timeoutMs);
	try {
		for (const [position, path] of curriculum.paths.entries()) {
			const courses = path.courses.map((reference) => courseFor(reference).id);
			const electiveNames = path.electives.map(
				(reference) => courseFor(reference).name,
			);
			const description = [
				path.description,
				electiveNames.length
					? `Optional courses: ${electiveNames.join("; ")}.`
					: "",
			]
				.filter(Boolean)
				.join("\n\n");
			const draft: LearningPathUpsertBody = {
				title: path.name,
				slug: path.slug,
				description,
				position,
				is_published: false,
			};
			let draftConfirmed = false;
			let publicationAttempted = false;
			try {
				const staged = await client.upsertLearningPath(path.id, draft);
				verifyMetadata(staged, path.id, draft);
				draftConfirmed = true;
				const extra = unexpectedSteps(staged, courses);
				if (extra.length)
					throw new Error(
						`Unexpected remote steps: ${extra.join(", ")}. Left as a draft; remove those steps deliberately before retrying.`,
					);
				for (const [stepPosition, courseId] of courses.entries())
					await client.upsertLearningPathStep(path.id, courseId, stepPosition);
				const saved = await client.getLearningPath(path.id);
				verifyMetadata(saved, path.id, draft);
				verifySteps(saved, courses);
				publicationAttempted = true;
				const published = { ...draft, is_published: true };
				const final = await client.upsertLearningPath(path.id, published);
				verifyMetadata(final, path.id, published);
				verifySteps(final, courses);
				result.paths.push({ id: path.id, status: "published" });
			} catch (error) {
				let message = errorMessage(error);
				if (publicationAttempted) {
					// A publication response can be lost after the write. Restore a draft with a fresh timeout.
					try {
						const recovery = createUniversityClient({
							baseUrl: options.apiUrl,
							pat: options.pat,
							signal: AbortSignal.timeout(10_000),
						});
						verifyMetadata(
							await recovery.upsertLearningPath(path.id, draft),
							path.id,
							draft,
						);
						draftConfirmed = true;
					} catch (cleanupError) {
						draftConfirmed = false;
						message += ` Could not confirm draft recovery: ${errorMessage(cleanupError)}`;
					}
				}
				result.paths.push({
					id: path.id,
					status: draftConfirmed ? "draft" : "failed",
					error: message,
				});
				result.errors.push(`${path.id}: ${message}`);
				if (controller.signal.aborted) break;
			}
		}
	} finally {
		clearTimeout(timeout);
	}
	result.passed =
		result.errors.length === 0 &&
		result.paths.length === curriculum.paths.length;
	return result;
}
