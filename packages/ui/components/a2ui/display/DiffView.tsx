"use client";

import {
	type DiffContentKind,
	type DiffViewMode,
	DiffViewer,
	type MarkdownMode,
} from "../../ui/diff-viewer";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, DiffViewComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIDiffView({
	elementRef,
	component,
	style,
}: ComponentProps<DiffViewComponent>) {
	const original = useResolved<string>(component.original) ?? "";
	const modified = useResolved<string>(component.modified) ?? "";
	const mode = useResolved<DiffViewMode>(component.mode);
	const kind = useResolved<DiffContentKind>(component.kind);
	const language = useResolved<string>(component.language);
	const markdownMode = useResolved<MarkdownMode>(component.markdownMode);
	const showLineNumbers = useResolved<boolean>(component.showLineNumbers);
	const wordWrap = useResolved<boolean>(component.wordWrap);
	const wordLevel = useResolved<boolean>(component.wordLevel);
	const collapseUnchanged = useResolved<boolean>(component.collapseUnchanged);
	const contextLines = useResolved<number>(component.contextLines);
	const showStats = useResolved<boolean>(component.showStats);
	const originalLabel = useResolved<string>(component.originalLabel);
	const modifiedLabel = useResolved<string>(component.modifiedLabel);
	const ignoreWhitespace = useResolved<boolean>(component.ignoreWhitespace);
	const ignoreCase = useResolved<boolean>(component.ignoreCase);
	const trimTrailingWhitespace = useResolved<boolean>(
		component.trimTrailingWhitespace,
	);
	const swapSides = useResolved<boolean>(component.swapSides);

	return (
		<DiffViewer
			ref={elementRef}
			original={original}
			modified={modified}
			mode={mode}
			kind={kind}
			language={language}
			markdownMode={markdownMode}
			showLineNumbers={showLineNumbers}
			wordWrap={wordWrap}
			wordLevel={wordLevel}
			collapseUnchanged={collapseUnchanged}
			contextLines={contextLines}
			showStats={showStats}
			originalLabel={originalLabel}
			modifiedLabel={modifiedLabel}
			ignoreWhitespace={ignoreWhitespace}
			ignoreCase={ignoreCase}
			trimTrailingWhitespace={trimTrailingWhitespace}
			swapSides={swapSides}
			className={resolveStyle(style)}
			style={resolveInlineStyle(style)}
		/>
	);
}
