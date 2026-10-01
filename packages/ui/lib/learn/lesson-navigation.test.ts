import { describe, expect, test } from "bun:test";
import { lessonNeighbors, remainingLessonMinutes } from "./lesson-navigation";

const lessons = [
	{ id: "intro", is_optional: false, estimated_minutes: 5 },
	{ id: "vision", is_optional: true, estimated_minutes: 20 },
	{ id: "build", is_optional: false, estimated_minutes: 10 },
	{ id: "deploy", is_optional: true, estimated_minutes: 15 },
	{ id: "final", is_optional: false, estimated_minutes: 5 },
];

describe("required and elective lesson paths", () => {
	test("moves between required lessons across interleaved electives", () => {
		expect(lessonNeighbors(lessons, "build")).toEqual({
			previous: lessons[0],
			next: lessons[4],
		});
		expect(lessonNeighbors(lessons, "final").next).toBeNull();
	});

	test("an explicitly opened elective can return to the course sequence", () => {
		expect(lessonNeighbors(lessons, "vision")).toEqual({
			previous: lessons[0],
			next: lessons[2],
		});
	});

	test("unknown lessons and empty courses have no navigation target", () => {
		expect(lessonNeighbors(lessons, "missing")).toEqual({
			previous: null,
			next: null,
		});
		expect(lessonNeighbors([], "intro")).toEqual({
			previous: null,
			next: null,
		});
	});

	test("finishing the core leaves zero required minutes and separate electives", () => {
		expect(
			remainingLessonMinutes(lessons, new Set(["intro", "build", "final"])),
		).toEqual({ required: 0, optional: 35 });
		expect(
			remainingLessonMinutes(lessons, new Set(["intro", "vision"])),
		).toEqual({ required: 15, optional: 15 });
	});
});
