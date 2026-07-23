import type { FlowPilotE2ERunReport } from "./types";

export interface FormatAppCreationReportOptions {
	includePassedChecks?: boolean;
}

export function formatAppCreationReport(
	report: FlowPilotE2ERunReport,
	options: FormatAppCreationReportOptions = {},
): string {
	const status = report.passed ? "PASS" : "FAIL";
	const lines = [
		`${status} ${report.caseId}: ${report.appName} (${report.appId || "no app id"})`,
		`model=${report.model.provider}/${report.model.model} reasoning=${report.model.reasoningEffort}`,
		`checks=${report.summary.passed}/${report.summary.checks} nodes=${report.inventory.totalNodes} pages=${report.inventory.pages} widgets=${report.inventory.widgets} tables=${report.inventory.tables} events=${report.inventory.events}`,
	];

	if (report.flowScript.authored) {
		lines.push(
			`authored=${report.flowScript.authored.nonWhitespaceCharacters} non-whitespace chars (~${report.flowScript.authored.estimatedTokens} tokens)`,
		);
	} else {
		lines.push("authored=missing");
	}
	for (const board of report.flowScript.canonical) {
		lines.push(
			`canonical[${board.boardId}]=${board.nonWhitespaceCharacters} non-whitespace chars (~${board.estimatedTokens} tokens)`,
		);
	}

	const renderedChecks = options.includePassedChecks
		? report.checks
		: report.failures;
	for (const result of renderedChecks) {
		lines.push(
			`- ${result.status === "pass" ? "PASS" : "FAIL"} [${result.code}] ${result.message}`,
		);
	}
	return lines.join("\n");
}

export function assertAppCreationCasePassed(
	report: FlowPilotE2ERunReport,
): asserts report is FlowPilotE2ERunReport & { passed: true } {
	if (!report.passed) {
		throw new Error(formatAppCreationReport(report));
	}
}

/** Stable failure signature for grouping repeated failures in a feedback loop. */
export function appCreationFailureFingerprint(
	report: FlowPilotE2ERunReport,
): string {
	const generatedIdPrefixes = [
		"boards.nonempty",
		"flowscript.canonical.board_present",
		"flowscript.lint.available",
		"flowscript.lint.errors",
		"flowscript.reconcile.available",
		"flowscript.reconcile.parse_valid",
		"flowscript.reconcile.reconcile_valid",
		"flowscript.reconcile.idempotent",
		"flowscript.reconcile.commands",
		"flowscript.reconcile.diagnostics",
		"integrity.page_board",
		"integrity.page_load_node",
		"integrity.page_unload_node",
		"integrity.page_interval_node",
		"integrity.page_widget",
		"integrity.page_reference",
		"integrity.event_board",
		"integrity.event_page",
		"integrity.event_node",
	];
	const codes = report.failures.map((failure) => {
		return (
			generatedIdPrefixes.find((prefix) =>
				failure.code.startsWith(`${prefix}.`),
			) ?? failure.code
		);
	});
	return [...new Set(codes)].sort().join("|");
}
