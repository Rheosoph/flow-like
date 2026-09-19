import type { WidgetBridge } from "@flow-like/widget-sdk";
import { type DocumentInfo, resolveExpectation } from "./location";
import { BLOCKED_EXPECTATION, NAVIGATION_CASES } from "./navigation";
import { type ProbeReport, check, createReport } from "./report";

const UNKNOWN_CASE = "nav.unknown";

/** Resolves once the host initialized the bridge, or the SDK fell back to standalone */
export function whenConnected(
	bridge: Pick<WidgetBridge, "$mode">,
): Promise<void> {
	if (bridge.$mode.get() !== "connecting") return Promise.resolve();
	return new Promise((resolve) => {
		const stop = bridge.$mode.listen((mode) => {
			if (mode === "connecting") return;
			stop();
			resolve();
		});
	});
}

/** Failure report for a document that a probe navigation should never have loaded */
export function reachedReport(
	widgetId: string,
	marker: string,
	info: DocumentInfo,
): ProbeReport {
	const caseId = NAVIGATION_CASES.some((entry) => entry.id === marker)
		? marker
		: UNKNOWN_CASE;
	return createReport({
		widgetId,
		phase: "navigation",
		expectation: resolveExpectation("auto", info, false),
		document: info.label,
		checks: {
			[caseId]: check(
				"fail",
				BLOCKED_EXPECTATION,
				`navigation reached ${info.label}`,
			),
		},
	});
}
