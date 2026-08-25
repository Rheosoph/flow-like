import { describe, expect, it } from "bun:test";
import {
	type ReaderChapter,
	type ReadingProgressRecord,
	bookmarkRecordId,
	mergePersistedReadingProgress,
	mergeReadingProgress,
	normalizeReadingPath,
	passageBookmarkRecordId,
	progressRecordId,
	summarizeReadingProgress,
} from "./reading-progress";

const chapters: ReaderChapter[] = [
	{
		entryId: "introduction",
		path: "/introduction/",
		title: "Intro",
		label: "Introduction",
	},
	{
		entryId: "part-1/chapter-1",
		path: "/part-1/chapter-1/",
		title: "One",
		label: "Chapter 1",
	},
	{
		entryId: "part-1/chapter-2",
		path: "/part-1/chapter-2/",
		title: "Two",
		label: "Chapter 2",
	},
];

function progress(
	entryId: string,
	percent: number,
	completed = false,
): ReadingProgressRecord {
	return {
		id: progressRecordId("edition", entryId),
		editionId: "edition",
		entryId,
		path: `/${entryId}/`,
		title: entryId,
		percent,
		furthestPercent: percent,
		scrollY: 100,
		headingId: "section",
		headingText: "Section",
		headingOffset: 12,
		updatedAt: "2026-01-01T00:00:00.000Z",
		completed,
	};
}

describe("reading path identity", () => {
	it("normalizes absolute URLs, duplicate slashes, and trailing slashes", () => {
		expect(
			normalizeReadingPath("https://book.flow-like.com/part-1//chapter-1?q=1"),
		).toBe("/part-1/chapter-1/");
		expect(normalizeReadingPath("introduction")).toBe("/introduction/");
		expect(normalizeReadingPath("/")).toBe("/");
	});

	it("namespaces progress and section bookmarks by edition", () => {
		expect(progressRecordId("open-2026", "introduction")).toBe(
			"open-2026:introduction",
		);
		expect(bookmarkRecordId("open-2026", "introduction", "why-flow")).toBe(
			"open-2026:introduction#why-flow",
		);
	});

	it("keeps passage bookmarks distinct inside the same section", () => {
		const first = passageBookmarkRecordId(
			"open-2026",
			"introduction",
			"why-flow",
			"First selected passage",
			0.25,
		);
		const second = passageBookmarkRecordId(
			"open-2026",
			"introduction",
			"why-flow",
			"Second selected passage",
			0.75,
		);

		expect(first).not.toBe(second);
		expect(
			passageBookmarkRecordId(
				"open-2026",
				"introduction",
				"why-flow",
				"  First   selected passage ",
				0.25,
			),
		).toBe(first);
	});
});

describe("mergeReadingProgress", () => {
	it("keeps the furthest point while updating the current location", () => {
		const previous = progress("introduction", 0.72);
		const next = mergeReadingProgress(previous, {
			...previous,
			percent: 0.3,
			scrollY: 250,
			headingId: "earlier",
			headingText: "Earlier",
			headingOffset: 8,
			updatedAt: "2026-01-02T00:00:00.000Z",
		});

		expect(next.percent).toBe(0.3);
		expect(next.furthestPercent).toBe(0.72);
		expect(next.headingId).toBe("earlier");
		expect(next.completed).toBe(false);
	});

	it("clamps invalid input and permanently marks a chapter complete", () => {
		const completed = mergeReadingProgress(undefined, {
			...progress("introduction", 1.4),
			percent: 1.4,
		});
		expect(completed.percent).toBe(1);
		expect(completed.furthestPercent).toBe(1);
		expect(completed.completed).toBe(true);
		expect(completed.completedAt).toBe(completed.updatedAt);

		const revisited = mergeReadingProgress(completed, {
			...completed,
			percent: Number.NaN,
			updatedAt: "2026-01-03T00:00:00.000Z",
		});
		expect(revisited.percent).toBe(0);
		expect(revisited.furthestPercent).toBe(1);
		expect(revisited.completed).toBe(true);
		expect(revisited.completedAt).toBe(completed.completedAt);
	});
});

describe("mergePersistedReadingProgress", () => {
	it("does not let a stale tab regress completion or the furthest point", () => {
		const completed = {
			...progress("introduction", 1, true),
			updatedAt: "2026-01-03T00:00:00.000Z",
			completedAt: "2026-01-03T00:00:00.000Z",
		};
		const staleTab = {
			...progress("introduction", 0.3),
			updatedAt: "2026-01-02T00:00:00.000Z",
		};

		const merged = mergePersistedReadingProgress(completed, staleTab);
		expect(merged.percent).toBe(1);
		expect(merged.furthestPercent).toBe(1);
		expect(merged.completed).toBe(true);
		expect(merged.completedAt).toBe(completed.completedAt);
	});

	it("uses the newest current location while retaining older furthest progress", () => {
		const furthest = progress("introduction", 0.8);
		const latest = {
			...progress("introduction", 0.4),
			updatedAt: "2026-01-04T00:00:00.000Z",
			headingId: "revisited",
		};

		const merged = mergePersistedReadingProgress(furthest, latest);
		expect(merged.percent).toBe(0.4);
		expect(merged.headingId).toBe("revisited");
		expect(merged.furthestPercent).toBe(0.8);
	});
});

describe("summarizeReadingProgress", () => {
	it("averages only canonical chapters and counts completed entries", () => {
		const summary = summarizeReadingProgress(chapters, [
			progress("introduction", 1, true),
			progress("part-1/chapter-1", 0.5),
			progress("not-in-the-book", 1, true),
		]);

		expect(summary.overallPercent).toBe(0.5);
		expect(summary.completedChapters).toBe(1);
		expect(summary.startedChapters).toBe(2);
	});

	it("returns an empty summary for an empty edition", () => {
		expect(summarizeReadingProgress([], [])).toEqual({
			overallPercent: 0,
			completedChapters: 0,
			startedChapters: 0,
		});
	});
});
