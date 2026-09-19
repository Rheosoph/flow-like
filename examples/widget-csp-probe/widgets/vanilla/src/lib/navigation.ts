import { type CanarySetup, canaryUrl } from "./canary";
import { RUNTIME_INPUTS, UNDECLARED_HOST } from "./hosts";
import {
	BASELINE_GRANT,
	type DocumentInfo,
	HISTORY_BACK_STEP,
	HISTORY_BACK_STEP_VALUE,
	RUNTIME_SEPARATOR,
	type RuntimeSlots,
	encodeRuntimeComponent,
	foreignGrant,
	withMarker,
} from "./location";
import { type ProbeCheck, check, describeError } from "./report";

export const SIBLING_WIDGET_ID = "csp-probe-sibling";
export const SHARED_SVG_PATH = "../../shared/csp-probe-marker.svg";
/** A navigation the engine silently cancels leaves this document running */
export const NAVIGATION_WAIT_MS = 5_000;
export const HISTORY_BACK_DELAY_MS = 800;

export const NAVIGATION_CASES = [
	{ id: "nav.baselineDocument", label: "Own baseline document (index.0.html)" },
	{ id: "nav.otherGrant", label: "Another grant filename" },
	{
		id: "nav.runtimeComponent",
		label: "Same grant, another runtime component (~…)",
	},
	{ id: "nav.sharedSvg", label: "Bundled SVG (../../shared/…svg)" },
	{ id: "nav.siblingWidget", label: "Sibling widget document" },
	{ id: "nav.blobDocument", label: "blob: document built by this widget" },
	{ id: "nav.dataDocument", label: "data: document" },
	{ id: "nav.replaceStateReload", label: "history.replaceState + reload" },
	{
		id: "nav.historyBack",
		label: "history.replaceState + navigate + history.back",
	},
] as const;

export type NavigationCaseId = (typeof NAVIGATION_CASES)[number]["id"];

export const BLOCKED_EXPECTATION =
	"blocked: the document stays, or the frame shows an engine error page";

const LOCAL_DOCUMENT_EXPECTATION =
	"blocked, or a document that inherits this policy: no request in the canary log and the red page reports the undeclared fetch blocked";

type LocalDocumentCase = "nav.blobDocument" | "nav.dataDocument";

function isLocalDocumentCase(
	caseId: NavigationCaseId,
): caseId is LocalDocumentCase {
	return caseId === "nav.blobDocument" || caseId === "nav.dataDocument";
}

export function navigationExpectation(caseId: NavigationCaseId): string {
	return isLocalDocumentCase(caseId)
		? LOCAL_DOCUMENT_EXPECTATION
		: BLOCKED_EXPECTATION;
}

export type NavigationStart =
	| { kind: "resolved"; check: ProbeCheck }
	/** `stayed` is the verdict when this document is still running after NAVIGATION_WAIT_MS */
	| { kind: "started"; stayed: ProbeCheck };

function siblingTarget(): URL {
	return new URL(
		`../${SIBLING_WIDGET_ID}/index.${BASELINE_GRANT}.html`,
		location.href,
	);
}

/** Runtime slots the host never approved: the undeclared host on a declared slot */
function forgedRuntimeComponent(current: string | null): string {
	const [first, second] = RUNTIME_INPUTS;
	const forged = (slot: string): RuntimeSlots => ({
		[slot]: [UNDECLARED_HOST.origin],
	});
	const candidate = encodeRuntimeComponent(forged(first));
	return candidate === current
		? encodeRuntimeComponent(forged(second))
		: candidate;
}

function directTarget(
	caseId: NavigationCaseId,
	info: DocumentInfo,
): URL | null {
	switch (caseId) {
		case "nav.baselineDocument":
			return info.kind === "grant"
				? new URL(`index.${BASELINE_GRANT}.html`, location.href)
				: null;
		case "nav.otherGrant":
			return new URL(`index.${foreignGrant(info)}.html`, location.href);
		case "nav.runtimeComponent":
			return info.grant === null
				? null
				: new URL(
						`index.${info.grant}${RUNTIME_SEPARATOR}${forgedRuntimeComponent(info.runtime)}.html`,
						location.href,
					);
		case "nav.sharedSvg":
			return new URL(SHARED_SVG_PATH, location.href);
		case "nav.siblingWidget":
			return siblingTarget();
		default:
			return null;
	}
}

function currentDocumentUrl(): URL {
	const url = new URL(location.href);
	url.hash = "";
	url.search = "";
	return url;
}

/** Rewrites the current history entry; engines should refuse a cross-document URL in an opaque origin */
function forgeHistoryEntry(caseId: NavigationCaseId): ProbeCheck | null {
	try {
		history.replaceState(
			history.state,
			"",
			withMarker(siblingTarget(), caseId),
		);
		return null;
	} catch (error) {
		return check(
			"pass",
			BLOCKED_EXPECTATION,
			`history.replaceState refused the sibling URL: ${describeError(error)}`,
		);
	}
}

function stayedAfter(
	action: string,
	expected = BLOCKED_EXPECTATION,
): ProbeCheck {
	return check(
		"pass",
		expected,
		`document still running ${NAVIGATION_WAIT_MS} ms after ${action}`,
	);
}

function scriptValue(value: string): string {
	return JSON.stringify(value).replaceAll("<", "\\u003c");
}

/**
 * A page without the SDK: it cannot report to the host, so it paints itself
 * red, shows whether it reached the undeclared host, and pings the canary.
 */
function localDocumentHtml(
	caseId: NavigationCaseId,
	scheme: string,
	canary: CanarySetup,
): string {
	const pings =
		canary.kind === "ready"
			? {
					fetch: canaryUrl(canary.run, caseId, "fetch"),
					img: canaryUrl(canary.run, caseId, "img.png"),
				}
			: null;
	return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>CSP probe navigation target</title></head>
<body style="margin:0;padding:12px;background:#b91c1c;color:#fff;font:600 14px system-ui,sans-serif">
<p>NAVIGATION REACHED: ${caseId} loaded a ${scheme}: document.</p>
<p id="result">Checking network access…</p>
<script>
(async () => {
	const out = document.getElementById("result");
	const lines = [];
	try {
		await fetch(${scriptValue(UNDECLARED_HOST.fetchUrl)}, { mode: "no-cors", cache: "no-store", credentials: "omit" });
		lines.push("fetch to ${UNDECLARED_HOST.origin} REACHED: this document lost the widget policy. FAIL.");
	} catch {
		lines.push("fetch to ${UNDECLARED_HOST.origin} blocked.");
	}
	const pings = ${pings === null ? "null" : `{ fetch: ${scriptValue(pings.fetch)}, img: ${scriptValue(pings.img)} }`};
	if (pings) {
		fetch(pings.fetch, { mode: "no-cors", cache: "no-store", credentials: "omit" }).catch(() => {});
		new Image().src = pings.img;
		lines.push("Canary pinged: any log entry under ${caseId}/ is a FAIL.");
	} else {
		lines.push("No canaryUrl was set, so only the line above decides.");
	}
	out.textContent = lines.join(" ");
})();
</script>
</body></html>`;
}

function localDocumentUrl(
	caseId: LocalDocumentCase,
	canary: CanarySetup,
): string {
	if (caseId === "nav.dataDocument") {
		return `data:text/html;charset=utf-8,${encodeURIComponent(localDocumentHtml(caseId, "data", canary))}`;
	}
	return URL.createObjectURL(
		new Blob([localDocumentHtml(caseId, "blob", canary)], {
			type: "text/html",
		}),
	);
}

/**
 * Starts the navigation for `caseId`. `announce` runs right before the
 * document may be replaced, so its report reaches the host first.
 */
export function startNavigation(
	caseId: NavigationCaseId,
	info: DocumentInfo,
	canary: CanarySetup,
	announce: (action: string) => void,
): NavigationStart {
	if (caseId === "nav.replaceStateReload" || caseId === "nav.historyBack") {
		const returnUrl = currentDocumentUrl();
		const refused = forgeHistoryEntry(caseId);
		if (refused) return { kind: "resolved", check: refused };
		if (caseId === "nav.replaceStateReload") {
			const action = "reloading the rewritten history entry";
			announce(action);
			location.reload();
			return { kind: "started", stayed: stayedAfter(action) };
		}
		returnUrl.searchParams.set(HISTORY_BACK_STEP, HISTORY_BACK_STEP_VALUE);
		announce("navigating to this document before calling history.back()");
		location.assign(returnUrl.href);
		return {
			kind: "started",
			stayed: check(
				"review",
				BLOCKED_EXPECTATION,
				"the navigation to this document's own URL did not complete, so history.back() never ran",
			),
		};
	}

	if (isLocalDocumentCase(caseId)) {
		const scheme = caseId === "nav.blobDocument" ? "blob:" : "data:";
		const action = `navigating to a ${scheme} document${canary.kind === "ready" ? ` (canary ${canary.run.prefix}/${caseId}/)` : ""}; a red page decides through its network line and the canary log`;
		announce(action);
		location.assign(localDocumentUrl(caseId, canary));
		return {
			kind: "started",
			stayed: stayedAfter(action, LOCAL_DOCUMENT_EXPECTATION),
		};
	}

	const target = directTarget(caseId, info);
	if (!target) {
		return {
			kind: "resolved",
			check: check(
				"skip",
				BLOCKED_EXPECTATION,
				"only meaningful from a granted document",
			),
		};
	}
	const action = `navigating to ${bundleRelative(target)}`;
	announce(action);
	location.assign(withMarker(target, caseId));
	return { kind: "started", stayed: stayedAfter(action) };
}

export function historyBackStayed(): ProbeCheck {
	return stayedAfter("history.back()");
}

function bundleRelative(url: URL): string {
	const path =
		/\/((?:widgets|shared)\/.*)$/.exec(url.pathname)?.[1] ?? url.pathname;
	return path.replace(/~[A-Za-z0-9_-]+(\.html)$/, "~{forged runtime}$1");
}
