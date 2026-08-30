export const READING_COMPLETE_THRESHOLD = 0.98;

export interface ReaderChapter {
	entryId: string;
	path: string;
	title: string;
	label: string;
}

export interface ReadingProgressRecord {
	id: string;
	editionId: string;
	entryId: string;
	path: string;
	title: string;
	percent: number;
	furthestPercent: number;
	scrollY: number;
	headingId: string;
	headingText: string;
	headingOffset: number;
	sectionProgress?: number;
	updatedAt: string;
	completed: boolean;
	completedAt?: string;
}

export interface ReadingBookmark {
	id: string;
	editionId: string;
	entryId: string;
	path: string;
	title: string;
	headingId: string;
	headingText: string;
	scrollY: number;
	headingOffset: number;
	sectionProgress?: number;
	percent: number;
	quote?: string;
	createdAt: string;
}

export interface ReadingComment {
	id: string;
	editionId: string;
	entryId: string;
	path: string;
	title: string;
	headingId: string;
	headingText: string;
	scrollY: number;
	headingOffset: number;
	sectionProgress?: number;
	percent: number;
	body: string;
	quote?: string;
	createdAt: string;
	updatedAt: string;
}

export interface ReadingSummary {
	overallPercent: number;
	completedChapters: number;
	startedChapters: number;
}

export function clampProgress(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(0, value));
}

export function normalizeReadingPath(value: string): string {
	let pathname = value.trim();
	try {
		pathname = new URL(pathname, "https://book.flow-like.local").pathname;
	} catch {
		pathname = "/";
	}

	pathname = `/${pathname}`.replace(/\/{2,}/g, "/");
	return pathname === "/" ? pathname : `${pathname.replace(/\/$/, "")}/`;
}

export function progressRecordId(editionId: string, entryId: string): string {
	return `${editionId}:${entryId}`;
}

export function bookmarkRecordId(
	editionId: string,
	entryId: string,
	headingId: string,
): string {
	return `${editionId}:${entryId}#${headingId || "_top"}`;
}

function stableBookmarkHash(value: string): string {
	let hash = 2166136261;
	for (let index = 0; index < value.length; index += 1) {
		hash ^= value.charCodeAt(index);
		hash = Math.imul(hash, 16777619);
	}
	return (hash >>> 0).toString(36);
}

export function passageBookmarkRecordId(
	editionId: string,
	entryId: string,
	headingId: string,
	quote: string,
	sectionProgress = 0,
): string {
	const normalizedQuote = quote.replace(/\s+/g, " ").trim();
	const position = Math.round(
		clampProgress(sectionProgress) * 100_000,
	).toString(36);
	return `${bookmarkRecordId(editionId, entryId, headingId)}@${position}-${stableBookmarkHash(normalizedQuote)}`;
}

export function mergeReadingProgress(
	previous: ReadingProgressRecord | undefined,
	next: Omit<
		ReadingProgressRecord,
		"furthestPercent" | "completed" | "completedAt"
	>,
): ReadingProgressRecord {
	const percent = clampProgress(next.percent);
	const furthestPercent = Math.max(
		clampProgress(previous?.furthestPercent ?? 0),
		percent,
	);
	const completed =
		Boolean(previous?.completed) ||
		furthestPercent >= READING_COMPLETE_THRESHOLD;

	return {
		...next,
		percent,
		furthestPercent,
		completed,
		completedAt: completed
			? (previous?.completedAt ?? next.updatedAt)
			: undefined,
	};
}

/**
 * Reconciles concurrent tab writes without allowing an older tab to move the
 * durable furthest point backwards or clear completion.
 */
export function mergePersistedReadingProgress(
	existing: ReadingProgressRecord | undefined,
	incoming: ReadingProgressRecord,
): ReadingProgressRecord {
	if (!existing) {
		return {
			...incoming,
			percent: clampProgress(incoming.percent),
			furthestPercent: clampProgress(incoming.furthestPercent),
		};
	}

	const latest =
		existing.updatedAt.localeCompare(incoming.updatedAt) > 0
			? existing
			: incoming;
	const furthestPercent = Math.max(
		clampProgress(existing.furthestPercent),
		clampProgress(incoming.furthestPercent),
	);
	const completed =
		existing.completed ||
		incoming.completed ||
		furthestPercent >= READING_COMPLETE_THRESHOLD;
	const completedAt = [existing.completedAt, incoming.completedAt]
		.filter((value): value is string => Boolean(value))
		.sort()[0];

	return {
		...latest,
		percent: clampProgress(latest.percent),
		furthestPercent,
		completed,
		completedAt: completed ? (completedAt ?? latest.updatedAt) : undefined,
	};
}

export function summarizeReadingProgress(
	chapters: readonly ReaderChapter[],
	progress: readonly ReadingProgressRecord[],
): ReadingSummary {
	if (chapters.length === 0) {
		return { overallPercent: 0, completedChapters: 0, startedChapters: 0 };
	}

	const byEntryId = new Map(progress.map((record) => [record.entryId, record]));
	let total = 0;
	let completedChapters = 0;
	let startedChapters = 0;

	for (const chapter of chapters) {
		const record = byEntryId.get(chapter.entryId);
		if (!record) continue;
		const furthest = clampProgress(record.furthestPercent);
		total += furthest;
		if (furthest > 0) startedChapters += 1;
		if (record.completed || furthest >= READING_COMPLETE_THRESHOLD) {
			completedChapters += 1;
		}
	}

	return {
		overallPercent: total / chapters.length,
		completedChapters,
		startedChapters,
	};
}
