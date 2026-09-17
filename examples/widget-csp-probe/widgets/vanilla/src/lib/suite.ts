import type { WidgetCsp } from "@flow-like/widget-sdk";
import {
	CONNECT_ONLY_HOST,
	GRANTED_HOST,
	NESTED_FRAME_URL,
	UNDECLARED_HOST,
	checkDeclaration,
} from "./hosts";
import {
	checkCspMeta,
	checkFrameTree,
	checkIpcProtocol,
	checkNoReferrer,
	checkOpaqueOrigin,
	checkParentChildren,
	checkTauriInternals,
	checkWebView2Bridge,
	checkWebkitMessageHandler,
} from "./isolation";
import {
	type ViolationWatch,
	evaluateOutcome,
	probeFetch,
	probeImage,
	probeNestedFrame,
	probeWorker,
} from "./network";
import {
	type ProbeCheck,
	type ProbeChecks,
	type ProbeExpectation,
	check,
} from "./report";

export interface SuiteOptions {
	csp: WidgetCsp | undefined;
	expectation: ProbeExpectation;
	frameSlot: HTMLElement;
	violations: ViolationWatch;
	onCheck(id: string, result: ProbeCheck): void;
}

const REQUEST_DONE = "request completed";

async function runWorkerChecks(
	expectation: ProbeExpectation,
	violations: ViolationWatch,
	record: (id: string, result: ProbeCheck) => void,
) {
	const workersAllowed = expectation !== "baseline";
	const networkAllowed = expectation === "granted";
	const worker = await probeWorker(
		{ granted: GRANTED_HOST.fetchUrl, undeclared: UNDECLARED_HOST.fetchUrl },
		violations,
	);
	record(
		"worker.start",
		evaluateOutcome(workersAllowed, worker.start, "blob worker started"),
	);
	const skipped = (expected: string) =>
		check("skip", expected, "no worker was started");
	const { granted, undeclared } = worker.results;
	record(
		"worker.fetchGranted",
		granted
			? evaluateOutcome(networkAllowed, granted, REQUEST_DONE)
			: skipped(networkAllowed ? "allowed" : "blocked by CSP"),
	);
	record(
		"worker.fetchUndeclared",
		undeclared
			? evaluateOutcome(false, undeclared, REQUEST_DONE)
			: skipped("blocked by CSP"),
	);
}

/** Runs every check that keeps this document alive, in a fixed order */
export async function runSuite({
	csp,
	expectation,
	frameSlot,
	violations,
	onCheck,
}: SuiteOptions): Promise<ProbeChecks> {
	const checks: ProbeChecks = {};
	const record = (id: string, result: ProbeCheck) => {
		checks[id] = result;
		onCheck(id, result);
	};
	const networkAllowed = expectation === "granted";

	record("setup.declaration", checkDeclaration(csp));
	record("document.opaqueOrigin", checkOpaqueOrigin());
	record("document.noReferrer", checkNoReferrer());
	record("document.cspMeta", checkCspMeta(expectation));

	record(
		"fetch.granted",
		evaluateOutcome(
			networkAllowed,
			await probeFetch(GRANTED_HOST.fetchUrl, violations),
			REQUEST_DONE,
		),
	);
	record(
		"fetch.connectOnly",
		evaluateOutcome(
			networkAllowed,
			await probeFetch(CONNECT_ONLY_HOST.fetchUrl, violations),
			REQUEST_DONE,
		),
	);
	record(
		"fetch.undeclared",
		evaluateOutcome(
			false,
			await probeFetch(UNDECLARED_HOST.fetchUrl, violations),
			REQUEST_DONE,
		),
	);
	record(
		"img.granted",
		evaluateOutcome(
			networkAllowed,
			await probeImage(GRANTED_HOST.imageUrl, violations),
			"image loaded",
		),
	);
	record(
		"img.connectOnly",
		evaluateOutcome(
			false,
			await probeImage(CONNECT_ONLY_HOST.imageUrl, violations),
			"image loaded",
		),
	);
	record(
		"img.undeclared",
		evaluateOutcome(
			false,
			await probeImage(UNDECLARED_HOST.imageUrl, violations),
			"image loaded",
		),
	);

	await runWorkerChecks(expectation, violations, record);

	record(
		"frame.nestedHttps",
		await probeNestedFrame(NESTED_FRAME_URL, frameSlot, violations),
	);
	record("frames.parentHasNoChildren", checkParentChildren());
	for (const [id, result] of Object.entries(checkFrameTree())) {
		record(id, result);
	}

	record("ipc.tauriInternals", await checkTauriInternals());
	record("ipc.webkitMessageHandler", checkWebkitMessageHandler());
	record("ipc.webview2", checkWebView2Bridge());
	record("ipc.customProtocol", await checkIpcProtocol(violations));

	return checks;
}
