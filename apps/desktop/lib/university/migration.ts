import { execFile } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { promisify } from "node:util";
import { discoverCoursePlans } from "./curriculum";
import type { UniversityPlan } from "./types";

const run = promisify(execFile);

export interface CourseSnapshot {
	plan: string;
	value: UniversityPlan;
}

interface EntityLocation {
	plan: string;
	courseId: string;
	moduleId?: string;
	lessonId?: string;
}

interface Entity {
	id: string;
	kind: "module" | "lesson" | "challenge" | "appLink" | "appRef";
	location: EntityLocation;
}

function index(snapshots: CourseSnapshot[]): Map<string, Entity> {
	const entities = new Map<string, Entity>();
	const add = (entity: Entity) => {
		if (entities.has(entity.id))
			throw new Error(`Cannot compare duplicate entity id ${entity.id}.`);
		entities.set(entity.id, entity);
	};
	for (const snapshot of snapshots) {
		const course = snapshot.value.course;
		const location = { plan: snapshot.plan, courseId: course.id };
		for (const link of course.appLinks ?? [])
			add({ id: link.id, kind: "appLink", location });
		for (const module of course.modules) {
			add({ id: module.id, kind: "module", location });
			const inModule = { ...location, moduleId: module.id };
			for (const lesson of module.lessons) {
				add({ id: lesson.id, kind: "lesson", location: inModule });
				const inLesson = { ...inModule, lessonId: lesson.id };
				for (const challenge of lesson.challenges ?? [])
					add({ id: challenge.id, kind: "challenge", location: inLesson });
				for (const ref of lesson.appRefs ?? [])
					add({ id: ref.id, kind: "appRef", location: inLesson });
			}
		}
	}
	return entities;
}

export function compareCourseEntities(
	before: CourseSnapshot[],
	after: CourseSnapshot[],
) {
	const old = index(before);
	const current = index(after);
	const removed: Entity[] = [];
	const moved: Array<{
		id: string;
		kind: Entity["kind"];
		from: EntityLocation;
		to: EntityLocation;
	}> = [];
	for (const entity of old.values()) {
		const next = current.get(entity.id);
		if (!next) removed.push(entity);
		else if (
			entity.kind !== next.kind ||
			entity.location.courseId !== next.location.courseId ||
			entity.location.moduleId !== next.location.moduleId ||
			entity.location.lessonId !== next.location.lessonId
		)
			moved.push({
				id: entity.id,
				kind: entity.kind,
				from: entity.location,
				to: next.location,
			});
	}
	removed.sort((left, right) => left.id.localeCompare(right.id));
	moved.sort((left, right) => left.id.localeCompare(right.id));
	return { removed, moved };
}

/** Reads Git objects and current plans; it never changes the repository or calls the API. */
export async function compareCurriculumRef(catalogPath: string, ref: string) {
	if (!ref || ref.startsWith("-") || /[\s\0]/u.test(ref))
		throw new Error(
			"Comparison ref must be a Git commit reference without whitespace.",
		);
	const catalog = resolve(catalogPath);
	const base = await realpath(dirname(catalog));
	const git = async (args: string[]) =>
		(
			await run("git", args, { cwd: base, maxBuffer: 16 * 1024 * 1024 })
		).stdout.trimEnd();
	const repository = await realpath(
		await git(["rev-parse", "--show-toplevel"]),
	);
	const commit = await git([
		"rev-parse",
		"--verify",
		"--end-of-options",
		`${ref}^{commit}`,
	]);
	const prefix = relative(repository, join(base, "courses"))
		.split(sep)
		.join("/");
	if (prefix.startsWith("../"))
		throw new Error("The curriculum must be inside its Git repository.");
	const historical = (
		await git([
			"-C",
			repository,
			"ls-tree",
			"-r",
			"--name-only",
			commit,
			"--",
			prefix,
		])
	)
		.split("\n")
		.filter((path) => path.endsWith("/course.plan.json"));
	const before = await Promise.all(
		historical.map(async (path) => ({
			plan: path,
			value: JSON.parse(
				await git(["show", `${commit}:${path}`]),
			) as UniversityPlan,
		})),
	);
	const files = await discoverCoursePlans(join(base, "courses"));
	const after = await Promise.all(
		files.map(async (path) => ({
			plan: relative(repository, path).split(sep).join("/"),
			value: JSON.parse(await readFile(path, "utf8")) as UniversityPlan,
		})),
	);
	return {
		schema: "flow-like.university-migration/v1",
		ref,
		commit,
		...compareCourseEntities(before, after),
	};
}
