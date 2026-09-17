import {
	BASELINE_GRANT,
	type DocumentInfo,
	HISTORY_BACK_STEP,
	HISTORY_BACK_STEP_VALUE,
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
	{ id: "nav.sharedSvg", label: "Bundled SVG (../../shared/…svg)" },
	{ id: "nav.siblingWidget", label: "Sibling widget document" },
	{ id: "nav.replaceStateReload", label: "history.replaceState + reload" },
	{
		id: "nav.historyBack",
		label: "history.replaceState + navigate + history.back",
	},
] as const;

export type NavigationCaseId = (typeof NAVIGATION_CASES)[number]["id"];

export const BLOCKED_EXPECTATION =
	"blocked: the document stays, or the frame shows an engine error page";

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

function stayedAfter(action: string): ProbeCheck {
	return check(
		"pass",
		BLOCKED_EXPECTATION,
		`document still running ${NAVIGATION_WAIT_MS} ms after ${action}`,
	);
}

/**
 * Starts the navigation for `caseId`. `announce` runs right before the
 * document may be replaced, so its report reaches the host first.
 */
export function startNavigation(
	caseId: NavigationCaseId,
	info: DocumentInfo,
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
	return /\/((?:widgets|shared)\/.*)$/.exec(url.pathname)?.[1] ?? url.pathname;
}
