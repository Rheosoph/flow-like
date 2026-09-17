import { defineWidget } from "@flow-like/widget-sdk";
import type { ProbeReport } from "../../lib/report";

interface Inputs {
	/** What the host should have granted; auto reads it from the document URL @default "auto" */
	expectation: "auto" | "granted" | "preview" | "baseline";
	/** Run the non-destructive checks as soon as the host initializes the widget @default false */
	autoRun: boolean;
}

interface Events {
	/** Pass/fail map of a suite run, a navigation attempt, or a navigation that reached this document */
	result: ProbeReport;
}

interface Queries {
	/** The most recent report; `checks` is empty before the first run */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getReport: { args: void; returns: ProbeReport };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "csp-probe",
	name: "CSP Probe",
	description:
		"Checks, inside the real webview, that approved network sources work, everything else is blocked, navigations stay pinned, frames stay isolated and IPC stays unreachable.",
	sizing: { defaultHeight: 640, resizable: true, maxHeight: 2400 },
	capabilities: { workers: true },
	csp: {
		connectSrc: ["https://httpbin.org", "https://httpbingo.org"],
		imgSrc: ["https://httpbin.org"],
	},
	dev: {
		fixtures: {
			granted: { expectation: "granted" },
			baseline: { expectation: "baseline" },
		},
	},
});
