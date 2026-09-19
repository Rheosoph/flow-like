import { defineWidget } from "@flow-like/widget-sdk";
import type { ProbeReport } from "../../lib/report";

interface Inputs {
	/** What the host should have granted; auto reads it from the document URL @default "auto" */
	expectation: "auto" | "granted" | "preview" | "baseline";
	/** Run the non-destructive checks as soon as the host initializes the widget @default false */
	autoRun: boolean;
	/** https URL of an image on a host no static source covers; approve it when the widget asks @default "" */
	runtimeApprovedUrl: string;
	/** https URL of an image on another uncovered host; choose Don't allow when the widget asks @default "" */
	runtimeRefusedUrl: string;
	/** https URL of a server whose request log you can read; local-scheme checks and blob:/data: navigations point absolute URLs below it @default "" */
	canaryUrl: string;
	/** blob: URLs from the sibling widget and the host page, which this document must not be able to read @default [] */
	foreignBlobUrls: string[];
}

interface Events {
	/** Pass/fail map of a suite run, a local-scheme run, a navigation attempt, or a navigation that reached this document */
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
		"Checks, inside the real webview, that approved network sources work, everything else is blocked, local data stays local, navigations stay pinned, frames stay isolated and IPC stays unreachable.",
	sizing: { defaultHeight: 640, resizable: true, maxHeight: 2400 },
	capabilities: { workers: true },
	csp: [
		{
			reason:
				"Requests two test services to check that allowed sites stay reachable",
			connectSrc: ["https://httpbin.org", "https://httpbingo.org"],
			imgSrc: ["https://httpbin.org"],
		},
		{
			reason:
				"Loads a test image from any Wikipedia subdomain to check wildcard matching",
			connectSrc: ["https://*.wikipedia.org"],
			imgSrc: ["https://*.wikipedia.org"],
		},
		{
			reason: "Loads test images from addresses given to it while it runs",
			inputs: [
				{ path: "runtimeApprovedUrl", directives: ["connectSrc", "imgSrc"] },
				{ path: "runtimeRefusedUrl", directives: ["connectSrc", "imgSrc"] },
			],
		},
	],
	dev: {
		fixtures: {
			granted: { expectation: "granted" },
			baseline: { expectation: "baseline" },
			runtime: {
				expectation: "granted",
				runtimeApprovedUrl:
					"https://upload.wikimedia.org/wikipedia/commons/7/70/Example.png",
				runtimeRefusedUrl:
					"https://www.gstatic.com/images/branding/product/1x/googleg_48dp.png",
			},
		},
	},
});
