/**
 * The channel a transport uses to tell a mounted Page that its contract no
 * longer matches the authority's, so the Page can refetch itself instead of
 * asking the user to reload.
 *
 * Deliberately NOT a `window` CustomEvent, unlike `board-sync-events`. A Page
 * renders author-controlled content, and an `iframe` component takes both
 * `srcdoc` and `sandbox` as bound props — an author who sets
 * `sandbox="allow-scripts allow-same-origin"` gets script execution in this
 * realm and could forge signals on `window`. Every consequence of that is a
 * denial-of-service: a forged signal makes Pages refetch and re-run their
 * onLoad workflows on a loop. A module-scoped registry is reachable only from
 * code that imports this module, which page content cannot do.
 */

export type PageContractFailure =
	| "stale_action"
	| "dead_grant"
	| "stale_manifest";

export interface PageContractDriftDetail {
	appId: string;
	eventId?: string;
	/** The revision the surface that raised this was rendered with. */
	renderedRevision?: string;
	reason: PageContractFailure | "missing_contract";
}

type Listener = (detail: PageContractDriftDetail) => void;

const listeners = new Set<Listener>();

/**
 * Signals are raised per rejected click, so a Page whose contract is genuinely
 * broken (a deleted onLoad node, a dead interval hook) would otherwise refetch
 * on every retry. The floor is per (app, event, reason) because two distinct
 * failures on one Page are two separate things to heal.
 */
const REJECTION_THROTTLE_MS = 5_000;
const lastRejection = new Map<string, number>();

export function subscribeToPageContractDrift(listener: Listener): () => void {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/**
 * The authority refused a Page trigger for a reason a refreshed contract can
 * cure. Raised only on failure, never after a successful run: a Page that just
 * executed has already had its surface rewritten by that run's A2UI messages,
 * and refetching on top of it would re-run onLoad over live content.
 */
export function notifyPageContractRejected(
	detail: PageContractDriftDetail,
): boolean {
	if (!detail.appId) return false;
	const key = `${detail.appId}|${detail.eventId ?? ""}|${detail.reason}`;
	const now = Date.now();
	if (now - (lastRejection.get(key) ?? 0) < REJECTION_THROTTLE_MS) return false;
	lastRejection.set(key, now);
	for (const listener of listeners) {
		try {
			listener(detail);
		} catch (error) {
			console.error("[pageContractDrift] listener threw:", error);
		}
	}
	return true;
}

/**
 * A signal with no `eventId` is an app-wide one and matches any event of that
 * app; a receiver that has not resolved its own event yet must NOT treat that
 * as a match, or one Page's failure reloads every unrelated surface.
 */
export function isPageContractDriftFor(
	detail: PageContractDriftDetail | undefined,
	appId: string | undefined | null,
	eventId: string | undefined | null,
): boolean {
	if (!detail?.appId || !appId || detail.appId !== appId) return false;
	if (!detail.eventId) return true;
	return Boolean(eventId) && detail.eventId === eventId;
}

/**
 * MIRROR SITES — these literals are produced by Rust. Rewording one there
 * disables healing here with no test failure and no error anywhere; the Rust
 * side carries a test pinning the exact strings.
 *   apps/desktop/src-tauri/src/functions/flow/run.rs           :83 :86 :110 :142 :150 :283 :292
 *   apps/desktop/src-tauri/src/local_page_actions.rs           :149 :164 :169 :173
 *   packages/api/src/routes/app/events/page_trigger.rs         :385 :515 :519 :617
 *
 * There is deliberately no bare "reload the Page" catch-all: that substring
 * also appears in routing refusals ("A local Page action cannot be sent to a
 * Remote Event; reload the Page"), which are configuration errors no refetch
 * can cure.
 */
const STALE_ACTION_MARKERS = [
	"The Page action is stale or invalid",
	"The Page action does not resolve to an executable entry",
];

const DEAD_GRANT_MARKERS = [
	"The local Page action expired",
	"The local Page action is unknown",
	"The local Page action does not belong to this Page execution",
	"The local Page action id is invalid",
	"A local Page action cannot carry a server capability",
	"This local Page action cannot execute on this device",
];

const STALE_MANIFEST_MARKERS = [
	"The Page manifest is stale",
	"The Page manifest revision is required",
	"This device holds no Page contract for this Event",
];

/**
 * Reads every shape a Page trigger failure actually arrives in.
 *
 * The native path matters most and is the easiest to miss: Tauri rejects
 * `invoke` with the *serialized* error value, so a `TauriFunctionError` reaches
 * JS as the plain object `{ error: "…" }` — not an `Error`, no `message`. The
 * hosted path throws `ApiResponseError`, which carries the server's text on
 * `serverMessage`.
 */
export function classifyPageContractError(
	error: unknown,
): PageContractFailure | null {
	const message = pageContractErrorMessage(error);
	if (!message) return null;
	if (STALE_ACTION_MARKERS.some((marker) => message.includes(marker)))
		return "stale_action";
	if (DEAD_GRANT_MARKERS.some((marker) => message.includes(marker)))
		return "dead_grant";
	if (STALE_MANIFEST_MARKERS.some((marker) => message.includes(marker)))
		return "stale_manifest";
	return null;
}

function pageContractErrorMessage(error: unknown): string {
	if (typeof error === "string") return error;
	const candidate = error as {
		serverMessage?: unknown;
		error?: unknown;
		message?: unknown;
	};
	if (typeof candidate?.serverMessage === "string")
		return candidate.serverMessage;
	// The Tauri `{ error: string }` rejection shape.
	if (typeof candidate?.error === "string") return candidate.error;
	if (typeof candidate?.message === "string") return candidate.message;
	return "";
}

/** Test seam — this module holds process-global throttle state. */
export function resetPageContractDrift(): void {
	lastRejection.clear();
	listeners.clear();
}
