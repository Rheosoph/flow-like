import { useTranslation } from "@flow-like/locales";
import type { IFlowScriptSourceSpan } from "../../../state/backend-state/board-state";
import {
	type FlowScriptDiagnosticEntry,
	sourceSpanToRange,
} from "./flowscript-diagnostics";

export function FlowScriptDiagnosticList({
	entries,
	source,
	onReveal,
}: Readonly<{
	entries: readonly FlowScriptDiagnosticEntry[];
	/** The submitted source, used to display editor columns even after the draft changes. */
	source?: string;
	/** Omitted when these coordinates no longer belong to the current editor buffer. */
	onReveal?: (span: IFlowScriptSourceSpan) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<ul className="space-y-1">
			{entries.map((entry, index) => (
				<li
					key={`${index}:${entry.message}`}
					className="whitespace-pre-wrap text-xs break-words"
				>
					{entry.scope && (
						<span className="mr-1 font-medium">
							{entry.scope.replace(/^(function|event):/, "$1 ")}:
						</span>
					)}
					{entry.message}
					{entry.spans.map((span, site) => {
						const range =
							source === undefined
								? undefined
								: sourceSpanToRange(source, span);
						const label = t("flowscriptDiagnosticLocation", {
							defaultValue: "Line {{line}}, column {{column}}",
							line: span.start.line,
							column: range?.startColumn ?? span.start.column,
						});
						return (
							<span
								key={`${site}:${span.start.line}:${span.start.column}`}
								className="ml-2 inline-block"
							>
								{!entry.exact && (
									<span className="mr-1">
										{t(
											"flowscriptDiagnosticPossibleLocation",
											"Possible location:",
										)}
									</span>
								)}
								{range && onReveal ? (
									<button
										type="button"
										className="underline underline-offset-2 hover:text-foreground focus-visible:outline focus-visible:outline-2"
										onClick={() => onReveal(span)}
									>
										{label}
									</button>
								) : (
									<span>{label}</span>
								)}
							</span>
						);
					})}
				</li>
			))}
		</ul>
	);
}
