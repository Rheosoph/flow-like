import type { WidgetCspPurpose } from "@flow-like/widget-sdk";
import {
	CONNECT_ONLY_HOST,
	GRANTED_HOST,
	NESTED_FRAME_URL,
	type ProbeEndpoint,
	RUNTIME_INPUTS,
	type RuntimeInput,
	UNDECLARED_HOST,
	WILDCARD_APEX,
	WILDCARD_SUBDOMAIN,
	checkDeclaration,
	declaredDirectivesFor,
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
import type { RuntimeSlots } from "./location";
import {
	type ViolationWatch,
	evaluateOutcome,
	probeFetch,
	probeImage,
	probeNestedFrame,
	probeWorker,
} from "./network";
import { originOf, servedDirectivesFor } from "./policy";
import {
	type ProbeCheck,
	type ProbeChecks,
	type ProbeExpectation,
	check,
} from "./report";

export interface SuiteOptions {
	csp: readonly WidgetCspPurpose[] | undefined;
	expectation: ProbeExpectation;
	runtimeUrls: Readonly<Record<RuntimeInput, string>>;
	/** Where the document keeps its runtime sources */
	document: { web: boolean; runtime: RuntimeSlots | null };
	frameSlot: HTMLElement;
	violations: ViolationWatch;
	onCheck(id: string, result: ProbeCheck): void;
}

type RecordCheck = (id: string, result: ProbeCheck) => void;

const REQUEST_DONE = "request completed";
const IMAGE_DONE = "image loaded";
const SERVED_DIRECTIVES = ["connect-src", "img-src"];

function withNote(result: ProbeCheck, note: string): ProbeCheck {
	return check(result.status, result.expected, `${result.observed}; ${note}`);
}

function servedNote(origin: string): string {
	const directives = servedDirectivesFor(origin, SERVED_DIRECTIVES);
	return directives.length > 0
		? `served ${directives.join(" and ")} cover ${origin}`
		: `served policy does not cover ${origin}`;
}

async function endpointChecks(
	prefix: string,
	endpoint: Pick<ProbeEndpoint, "fetchUrl" | "imageUrl">,
	expectAllowed: boolean,
	violations: ViolationWatch,
	note: (result: ProbeCheck) => string,
	record: RecordCheck,
) {
	const fetched = evaluateOutcome(
		expectAllowed,
		await probeFetch(endpoint.fetchUrl, violations),
		REQUEST_DONE,
	);
	record(`${prefix}Fetch`, withNote(fetched, note(fetched)));
	const image = evaluateOutcome(
		expectAllowed,
		await probeImage(endpoint.imageUrl, violations),
		IMAGE_DONE,
	);
	record(`${prefix}Img`, withNote(image, note(image)));
}

/** `*.wikipedia.org` must cover `www.` and never the apex (WebKit before 246729@main matched it) */
async function runWildcardChecks(
	networkAllowed: boolean,
	violations: ViolationWatch,
	record: RecordCheck,
) {
	await endpointChecks(
		"wildcard.subdomain",
		WILDCARD_SUBDOMAIN,
		networkAllowed,
		violations,
		() => servedNote(WILDCARD_SUBDOMAIN.origin),
		record,
	);
	await endpointChecks(
		"wildcard.apex",
		WILDCARD_APEX,
		false,
		violations,
		(result) =>
			result.status === "fail"
				? `${servedNote(WILDCARD_APEX.origin)}, yet the engine matched the wildcard against its apex: add a widget_engine_gates.json row denying wildcardSources`
				: servedNote(WILDCARD_APEX.origin),
		record,
	);
}

function runtimeCarrier(options: SuiteOptions["document"]): string {
	if (options.runtime !== null) {
		return `document runtime component ${JSON.stringify(options.runtime)}`;
	}
	return options.web
		? "document URL carries no runtime component"
		: "desktop keeps runtime sources in its grant registry";
}

const RUNTIME_ROWS: Record<
	RuntimeInput,
	{ prefix: string; approve: boolean; setup: string }
> = {
	runtimeApprovedUrl: {
		prefix: "runtime.approved",
		approve: true,
		setup: "approve it when the widget asks",
	},
	runtimeRefusedUrl: {
		prefix: "runtime.refused",
		approve: false,
		setup: "choose Don't allow when the widget asks",
	},
};

/** A network input on a host the viewer approved at runtime vs one the viewer refused */
async function runRuntimeChecks(
	{
		csp,
		expectation,
		runtimeUrls,
		document: carrier,
		violations,
	}: SuiteOptions,
	record: RecordCheck,
) {
	for (const input of RUNTIME_INPUTS) {
		const row = RUNTIME_ROWS[input];
		const url = runtimeUrls[input].trim();
		const expectAllowed = expectation === "granted" && row.approve;
		const expected = expectAllowed ? "allowed" : "blocked by CSP";
		const origin = originOf(url);
		const declared = origin ? declaredDirectivesFor(csp, origin) : [];
		const skip =
			origin === null
				? `set ${input} to an https URL of an image on a host no static source covers, then ${row.setup}`
				: declared.length > 0
					? `${origin} is covered by static ${declared.join(", ")} sources; use a host no static source covers`
					: null;
		if (skip !== null) {
			record(`${row.prefix}Fetch`, check("skip", expected, skip));
			record(`${row.prefix}Img`, check("skip", expected, skip));
			continue;
		}
		const note = `${servedNote(origin ?? url)}; ${runtimeCarrier(carrier)}`;
		await endpointChecks(
			row.prefix,
			{ fetchUrl: url, imageUrl: url },
			expectAllowed,
			violations,
			() => note,
			record,
		);
	}
}

async function runWorkerChecks(
	expectation: ProbeExpectation,
	violations: ViolationWatch,
	record: RecordCheck,
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
export async function runSuite(options: SuiteOptions): Promise<ProbeChecks> {
	const { csp, expectation, frameSlot, violations, onCheck } = options;
	const checks: ProbeChecks = {};
	const record: RecordCheck = (id, result) => {
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
			IMAGE_DONE,
		),
	);
	record(
		"img.connectOnly",
		evaluateOutcome(
			false,
			await probeImage(CONNECT_ONLY_HOST.imageUrl, violations),
			IMAGE_DONE,
		),
	);
	record(
		"img.undeclared",
		evaluateOutcome(
			false,
			await probeImage(UNDECLARED_HOST.imageUrl, violations),
			IMAGE_DONE,
		),
	);

	await runWildcardChecks(networkAllowed, violations, record);
	await runRuntimeChecks(options, record);
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
