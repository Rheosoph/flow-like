export { DiffViewer, type DiffViewerProps } from "./DiffViewer";
export {
	collapseContext,
	computeBlockDiff,
	computeDiff,
	computeInlineSegments,
	splitMarkdownBlocks,
	trimTrailingLines,
} from "./compute";
export type {
	DiffContentKind,
	DiffItem,
	DiffOptions,
	DiffResult,
	DiffRow,
	DiffSegment,
	DiffStats,
	DiffViewMode,
	MarkdownMode,
	RenderedBlock,
} from "./types";
