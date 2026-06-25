import { diffArrays, diffLines, diffWordsWithSpace } from "diff";
import type {
	DiffCell,
	DiffItem,
	DiffOptions,
	DiffResult,
	DiffRow,
	DiffSegment,
	RenderedBlock,
} from "./types";

function splitLines(value: string): string[] {
	const lines = value.split("\n");
	if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
	return lines;
}

export function trimTrailingLines(value: string): string {
	return value
		.split("\n")
		.map((line) => line.replace(/[ \t]+$/, ""))
		.join("\n");
}

function wordSegments(
	original: string,
	modified: string,
	ignoreCase: boolean,
): { left: DiffSegment[]; right: DiffSegment[] } {
	const parts = diffWordsWithSpace(original, modified, { ignoreCase });
	const left: DiffSegment[] = [];
	const right: DiffSegment[] = [];
	for (const part of parts) {
		if (part.added) {
			right.push({ text: part.value, kind: "added" });
		} else if (part.removed) {
			left.push({ text: part.value, kind: "removed" });
		} else {
			left.push({ text: part.value, kind: "common" });
			right.push({ text: part.value, kind: "common" });
		}
	}
	return { left, right };
}

function emptyCell(): DiffCell {
	return { lineNo: null, text: null };
}

/**
 * Computes a row-aligned diff suitable for split and unified rendering.
 * Each "change" row pairs a removed line with an added line so that
 * intra-line (word-level) highlighting can be applied to both sides.
 */
export function computeDiff(
	originalInput: string,
	modifiedInput: string,
	options: DiffOptions = {},
): DiffResult {
	const {
		wordLevel = true,
		ignoreWhitespace = false,
		ignoreCase = false,
		trimTrailingWhitespace = false,
	} = options;

	const original = trimTrailingWhitespace
		? trimTrailingLines(originalInput)
		: originalInput;
	const modified = trimTrailingWhitespace
		? trimTrailingLines(modifiedInput)
		: modifiedInput;

	const changes = diffLines(original, modified, {
		ignoreWhitespace,
		ignoreCase,
	});

	const rows: DiffRow[] = [];
	let leftNo = 1;
	let rightNo = 1;
	let additions = 0;
	let deletions = 0;
	let unchanged = 0;

	let i = 0;
	while (i < changes.length) {
		const change = changes[i];

		if (!change.added && !change.removed) {
			for (const line of splitLines(change.value)) {
				rows.push({
					type: "context",
					left: { lineNo: leftNo++, text: line },
					right: { lineNo: rightNo++, text: line },
				});
				unchanged++;
			}
			i++;
			continue;
		}

		let removedLines: string[] = [];
		let addedLines: string[] = [];

		if (change.removed) {
			removedLines = splitLines(change.value);
			i++;
			if (i < changes.length && changes[i].added) {
				addedLines = splitLines(changes[i].value);
				i++;
			}
		} else {
			addedLines = splitLines(change.value);
			i++;
		}

		deletions += removedLines.length;
		additions += addedLines.length;

		const pairCount = Math.min(removedLines.length, addedLines.length);
		for (let p = 0; p < pairCount; p++) {
			const segs = wordLevel
				? wordSegments(removedLines[p], addedLines[p], ignoreCase)
				: undefined;
			rows.push({
				type: "change",
				left: { lineNo: leftNo++, text: removedLines[p], segments: segs?.left },
				right: {
					lineNo: rightNo++,
					text: addedLines[p],
					segments: segs?.right,
				},
			});
		}
		for (let p = pairCount; p < removedLines.length; p++) {
			rows.push({
				type: "delete",
				left: { lineNo: leftNo++, text: removedLines[p] },
				right: emptyCell(),
			});
		}
		for (let p = pairCount; p < addedLines.length; p++) {
			rows.push({
				type: "insert",
				left: emptyCell(),
				right: { lineNo: rightNo++, text: addedLines[p] },
			});
		}
	}

	return { rows, stats: { additions, deletions, unchanged } };
}

/**
 * Collapses long runs of unchanged "context" rows into expandable gaps,
 * keeping `contextLines` of context on either side of every change.
 */
export function collapseContext(
	rows: DiffRow[],
	contextLines: number,
): DiffItem[] {
	const items: DiffItem[] = [];
	const context = Math.max(0, contextLines);
	let run: { row: DiffRow; index: number }[] = [];

	const flushRun = () => {
		if (run.length === 0) return;
		if (run.length <= context * 2 + 1) {
			for (const entry of run) {
				items.push({ kind: "row", row: entry.row, rowIndex: entry.index });
			}
		} else {
			const head = run.slice(0, context);
			const tail = run.slice(run.length - context);
			const hidden = run.slice(context, run.length - context);
			for (const entry of head) {
				items.push({ kind: "row", row: entry.row, rowIndex: entry.index });
			}
			items.push({
				kind: "gap",
				gapId: `gap-${hidden[0].index}-${hidden[hidden.length - 1].index}`,
				hiddenRows: hidden.map((entry) => entry.row),
				firstRowIndex: hidden[0].index,
			});
			for (const entry of tail) {
				items.push({ kind: "row", row: entry.row, rowIndex: entry.index });
			}
		}
		run = [];
	};

	rows.forEach((row, index) => {
		if (row.type === "context") {
			run.push({ row, index });
		} else {
			flushRun();
			items.push({ kind: "row", row, rowIndex: index });
		}
	});
	flushRun();

	return items;
}

/**
 * Splits markdown into top-level blocks separated by blank lines, keeping
 * fenced code blocks (``` / ~~~) intact so they diff and render as one unit.
 */
export function splitMarkdownBlocks(value: string): string[] {
	const blocks: string[] = [];
	let current: string[] = [];
	let fence: string | null = null;

	const flush = () => {
		const text = current.join("\n").replace(/\n+$/, "");
		if (text.trim().length > 0) blocks.push(text);
		current = [];
	};

	for (const line of value.split("\n")) {
		const fenceMatch = /^\s*(```|~~~)/.exec(line);
		if (fenceMatch) {
			current.push(line);
			if (fence === null) {
				fence = fenceMatch[1];
			} else if (line.trim().startsWith(fence)) {
				fence = null;
				flush();
			}
			continue;
		}
		if (fence === null && line.trim() === "") {
			flush();
			continue;
		}
		current.push(line);
	}
	flush();
	return blocks;
}

/**
 * Block-aligned diff used by the rendered markdown view: each row pairs a
 * removed block with an added block so changed blocks can be tinted and
 * rendered side by side.
 */
export function computeBlockDiff(
	originalInput: string,
	modifiedInput: string,
	options: DiffOptions = {},
): RenderedBlock[] {
	const { ignoreCase = false } = options;
	const norm = (value: string) =>
		(ignoreCase ? value.toLowerCase() : value).trim();

	const parts = diffArrays(
		splitMarkdownBlocks(originalInput),
		splitMarkdownBlocks(modifiedInput),
		{ comparator: (a, b) => norm(a) === norm(b) },
	);

	const result: RenderedBlock[] = [];
	let i = 0;
	while (i < parts.length) {
		const part = parts[i];

		if (!part.added && !part.removed) {
			for (const value of part.value) {
				result.push({ type: "context", left: value, right: value });
			}
			i++;
			continue;
		}

		let removed: string[] = [];
		let added: string[] = [];

		if (part.removed) {
			removed = part.value;
			i++;
			if (i < parts.length && parts[i].added) {
				added = parts[i].value;
				i++;
			}
		} else {
			added = part.value;
			i++;
		}

		const pairCount = Math.min(removed.length, added.length);
		for (let p = 0; p < pairCount; p++) {
			result.push({ type: "change", left: removed[p], right: added[p] });
		}
		for (let p = pairCount; p < removed.length; p++) {
			result.push({ type: "delete", left: removed[p], right: null });
		}
		for (let p = pairCount; p < added.length; p++) {
			result.push({ type: "insert", left: null, right: added[p] });
		}
	}

	return result;
}

/**
 * Whole-text word diff used by the flowing "inline" (tracked-changes) view.
 */
export function computeInlineSegments(
	originalInput: string,
	modifiedInput: string,
	options: DiffOptions = {},
): DiffSegment[] {
	const { ignoreCase = false, trimTrailingWhitespace = false } = options;
	const original = trimTrailingWhitespace
		? trimTrailingLines(originalInput)
		: originalInput;
	const modified = trimTrailingWhitespace
		? trimTrailingLines(modifiedInput)
		: modifiedInput;

	return diffWordsWithSpace(original, modified, { ignoreCase }).map((part) => ({
		text: part.value,
		kind: part.added ? "added" : part.removed ? "removed" : "common",
	}));
}
