export interface BookEditionEntry {
	readonly entryId: string;
	readonly anchor: string;
}

export interface BookEditionChapter extends BookEditionEntry {
	readonly number: number;
}

export interface BookEditionPart {
	readonly id: string;
	readonly anchor: string;
	readonly label: string;
	readonly title: string;
	readonly description: string;
	readonly chapters: readonly BookEditionChapter[];
}

export interface BookEdition {
	readonly id: string;
	readonly language: string;
	readonly title: string;
	readonly subtitle: string;
	readonly description: string;
	readonly publisher: string;
	readonly editionLabel: string;
	readonly year: number;
	readonly introduction: BookEditionEntry;
	readonly parts: readonly BookEditionPart[];
}

/**
 * The source of truth for every entry included in the current printable edition.
 *
 * Titles and chapter descriptions intentionally remain in content frontmatter so the
 * web edition, print edition, and generated table of contents cannot drift apart.
 */
export const CURRENT_BOOK_EDITION = {
	id: "flowbook-open-2026",
	language: "en",
	title: "FlowBook",
	subtitle: "A Developer's Guide to Flow-Like",
	description: "Build reliable software as typed text and a visible workflow.",
	publisher: "Flow-Like",
	editionLabel: "Open edition · 2026",
	year: 2026,
	introduction: {
		entryId: "introduction",
		anchor: "introduction",
	},
	parts: [
		{
			id: "part-1",
			anchor: "part-i",
			label: "Part I",
			title: "Software That Explains Itself",
			description:
				"Begin with the founding incident, establish the operating principles, and build the first Flow through both authoring views.",
			chapters: [
				{
					number: 1,
					entryId: "part-1/01-the-3-am-call",
					anchor: "chapter-01",
				},
				{
					number: 2,
					entryId: "part-1/02-manifesto-constrained-freedom",
					anchor: "chapter-02",
				},
				{
					number: 3,
					entryId: "part-1/03-one-platform-one-flow-model",
					anchor: "chapter-03",
				},
				{
					number: 4,
					entryId: "part-1/04-first-flow-incident-triage",
					anchor: "chapter-04",
				},
			],
		},
		{
			id: "part-2",
			anchor: "part-ii",
			label: "Part II",
			title: "Thinking and Writing in Flows",
			description:
				"Learn the language through the graph it represents: values, operations, control flow, state, and governed runtime boundaries.",
			chapters: [
				{
					number: 5,
					entryId: "part-2/05-nodes-pins-wires-execution",
					anchor: "chapter-05",
				},
				{
					number: 6,
					entryId: "part-2/06-anatomy-of-a-flowscript-document",
					anchor: "chapter-06",
				},
				{
					number: 7,
					entryId: "part-2/07-values-types-collections-interfaces",
					anchor: "chapter-07",
				},
				{
					number: 8,
					entryId: "part-2/08-calling-the-node-library",
					anchor: "chapter-08",
				},
				{
					number: 9,
					entryId: "part-2/09-expressions-operators-readable-sugar",
					anchor: "chapter-09",
				},
				{
					number: 10,
					entryId: "part-2/10-branches-loops-parallelism-return",
					anchor: "chapter-10",
				},
				{
					number: 11,
					entryId: "part-2/11-state-configuration-runtime-values-secrets",
					anchor: "chapter-11",
				},
				{
					number: 12,
					entryId: "part-2/12-functions-layers-handlers-caching",
					anchor: "chapter-12",
				},
				{
					number: 13,
					entryId: "part-2/13-events-interfaces-complete-apps",
					anchor: "chapter-13",
				},
			],
		},
		{
			id: "part-3",
			anchor: "part-iii",
			label: "Part III",
			title: "The Two-Way Contract",
			description:
				"Open the machinery that keeps the visual Board and canonical FlowScript aligned through typed meaning and guarded changes.",
			chapters: [
				{
					number: 14,
					entryId: "part-3/14-board-ast-text",
					anchor: "chapter-14",
				},
			],
		},
	],
} as const satisfies BookEdition;

export type CurrentBookEdition = typeof CURRENT_BOOK_EDITION;
