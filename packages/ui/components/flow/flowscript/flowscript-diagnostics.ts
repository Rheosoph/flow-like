import type {
	IFlowScriptSourceSpan,
	IFlowScriptStructuredDiagnostic,
} from "../../../state/backend-state/board-state";

export const FLOWSCRIPT_RECONCILE_DIAGNOSTIC_OWNER = "flowscript-reconcile";

export interface FlowScriptDiagnosticEntry {
	message: string;
	scope?: string;
	spans: readonly IFlowScriptSourceSpan[];
	exact: boolean;
}

/** Keep legacy-only errors visible alongside the compiler's located diagnostic groups. */
export function flowScriptDiagnosticEntries(
	diagnostics: readonly string[],
	structuredDiagnostics?: readonly IFlowScriptStructuredDiagnostic[],
): FlowScriptDiagnosticEntry[] {
	const matched = new Set<string>();
	const entries: FlowScriptDiagnosticEntry[] = [];
	for (const diagnostic of structuredDiagnostics ?? []) {
		const messages = [
			diagnostic.message,
			...(diagnostic.related_messages ?? []),
		];
		if (!messages.some((message) => diagnostics.includes(message))) continue;
		for (const message of messages) matched.add(message);
		entries.push({
			message: diagnostic.message,
			scope: diagnostic.scope,
			spans: diagnostic.spans?.length
				? diagnostic.spans
				: diagnostic.source_span
					? [diagnostic.source_span]
					: [],
			exact: diagnostic.source_span_is_exact === true,
		});
	}
	for (const message of diagnostics) {
		if (!matched.has(message))
			entries.push({ message, spans: [], exact: false });
	}
	return entries;
}

/** Rust source columns count Unicode scalars; Monaco columns count UTF-16 code units. */
export function sourceSpanToRange(source: string, span: IFlowScriptSourceSpan) {
	const lines = source.split("\n");
	const column = (lineNumber: number, scalarColumn: number) => {
		if (!Number.isInteger(lineNumber) || !Number.isInteger(scalarColumn))
			return;
		const line = lines[lineNumber - 1];
		if (line === undefined || scalarColumn < 1) return;
		const chars = Array.from(line);
		if (scalarColumn > chars.length + 1) return;
		return chars.slice(0, scalarColumn - 1).join("").length + 1;
	};
	const startColumn = column(span.start.line, span.start.column);
	const endColumn = column(span.end.line, span.end.column);
	if (startColumn === undefined || endColumn === undefined) return;
	if (
		span.end.line < span.start.line ||
		(span.end.line === span.start.line && endColumn < startColumn)
	)
		return;
	const empty = span.start.line === span.end.line && startColumn === endColumn;
	const nextChar = Array.from(
		lines[span.start.line - 1].slice(startColumn - 1),
	)[0];
	return {
		startLineNumber: span.start.line,
		startColumn,
		endLineNumber: span.end.line,
		endColumn: empty ? endColumn + (nextChar?.length ?? 1) : endColumn,
	};
}
