import { defineWidget } from "@flow-like/widget-sdk";
import type { ProbeReport } from "../../lib/report";

interface Inputs {
	/** Text shown while no probe navigation reached this document @default "CSP probe sibling: no permissions" */
	label: string;
}

interface Events {
	/** Failure report when a CSP probe navigation reached this document */
	result: ProbeReport;
}

export default defineWidget<Inputs, Events>({
	id: "csp-probe-sibling",
	name: "CSP Probe Sibling",
	description:
		"Navigation target for the CSP probe. Declares no capabilities and no network sources; mount it next to the probe to test cross-widget isolation.",
	sizing: { defaultHeight: 120, resizable: true },
});
