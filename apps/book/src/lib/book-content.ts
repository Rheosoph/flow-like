import { getCollection } from "astro:content";

const CHAPTER_ID_PATTERN = /^part-(\d+)\/(\d+)-/;

export interface BookContentChapter {
	readonly entryId: string;
	readonly number: number;
	readonly partId: string;
	readonly partNumber: number;
}

export interface BookContentPart {
	readonly id: string;
	readonly number: number;
	readonly chapters: readonly BookContentChapter[];
	readonly chapterRange: string;
	readonly firstEntryId: string;
}

export interface BookContentStats {
	readonly chapterCount: number;
	readonly partCount: number;
	readonly firstChapterNumber?: number;
	readonly lastChapterNumber?: number;
	readonly parts: readonly BookContentPart[];
}

const formatChapterNumber = (chapterNumber: number) =>
	String(chapterNumber).padStart(2, "0");

export async function getBookContentStats(): Promise<BookContentStats> {
	const entries = await getCollection("docs");
	const chapters = entries
		.flatMap((entry) => {
			const match = CHAPTER_ID_PATTERN.exec(entry.id);
			if (!match) return [];

			const partNumber = Number(match[1]);
			const chapterNumber = Number(match[2]);
			if (!Number.isInteger(partNumber) || !Number.isInteger(chapterNumber))
				return [];

			return [
				{
					entryId: entry.id,
					number: chapterNumber,
					partId: `part-${partNumber}`,
					partNumber,
				} satisfies BookContentChapter,
			];
		})
		.sort((left, right) => left.number - right.number);

	const chaptersByPart = Map.groupBy(chapters, (chapter) => chapter.partNumber);
	const parts = Array.from(chaptersByPart.entries())
		.sort(([left], [right]) => left - right)
		.flatMap(([partNumber, partChapters]) => {
			const firstChapter = partChapters.at(0);
			const lastChapter = partChapters.at(-1);
			if (!firstChapter || !lastChapter) return [];

			return [
				{
					id: `part-${partNumber}`,
					number: partNumber,
					chapters: partChapters,
					chapterRange:
						firstChapter.number === lastChapter.number
							? formatChapterNumber(firstChapter.number)
							: `${formatChapterNumber(firstChapter.number)}–${formatChapterNumber(lastChapter.number)}`,
					firstEntryId: firstChapter.entryId,
				} satisfies BookContentPart,
			];
		});

	return {
		chapterCount: chapters.length,
		partCount: parts.length,
		firstChapterNumber: chapters.at(0)?.number,
		lastChapterNumber: chapters.at(-1)?.number,
		parts,
	};
}
