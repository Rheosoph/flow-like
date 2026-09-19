import { defineWidget } from "@flow-like/widget-sdk";
import type { SiblingBlobUrls } from "../../lib/blobs";
import type { ProbeReport } from "../../lib/report";

interface Inputs {
	/** Text shown while no probe navigation reached this document @default "CSP probe sibling: no permissions" */
	label: string;
}

interface Events {
	/** Failure report when a CSP probe navigation reached this document */
	result: ProbeReport;
	/** blob: URLs this document holds, for the probe's foreignBlobUrls input; sent once per mount */
	blobUrls: SiblingBlobUrls;
}

interface Queries {
	/** blob: URLs this document holds while it stays mounted */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getBlobUrls: { args: void; returns: SiblingBlobUrls };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "csp-probe-sibling",
	name: "CSP Probe Sibling",
	description:
		"Navigation target and blob: URL source for the CSP probe. Declares no capabilities and no network sources; mount it next to the probe to test cross-widget isolation.",
	sizing: { defaultHeight: 180, resizable: true },
});
