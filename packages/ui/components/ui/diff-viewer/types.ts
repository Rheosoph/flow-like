export type DiffViewMode = "split" | "unified" | "inline";

export type DiffContentKind =
	| "auto"
	| "text"
	| "code"
	| "markdown"
	| "json"
	| "document";

export type MarkdownMode = "source" | "rendered";

export interface DiffOptions {
	wordLevel?: boolean;
	ignoreWhitespace?: boolean;
	ignoreCase?: boolean;
	trimTrailingWhitespace?: boolean;
}

export type DiffSegmentKind = "common" | "added" | "removed";

export interface DiffSegment {
	text: string;
	kind: DiffSegmentKind;
}

export interface DiffCell {
	lineNo: number | null;
	text: string | null;
	segments?: DiffSegment[];
}

export type DiffRowType = "context" | "change" | "insert" | "delete";

export interface DiffRow {
	type: DiffRowType;
	left: DiffCell;
	right: DiffCell;
}

export interface DiffStats {
	additions: number;
	deletions: number;
	unchanged: number;
}

export interface DiffResult {
	rows: DiffRow[];
	stats: DiffStats;
}

export type DiffItem =
	| { kind: "row"; row: DiffRow; rowIndex: number }
	| {
			kind: "gap";
			gapId: string;
			hiddenRows: DiffRow[];
			firstRowIndex: number;
	  };

export interface RenderedBlock {
	type: DiffRowType;
	left: string | null;
	right: string | null;
}
