/**
 * Client-side sampling for anonymous usage events.
 *
 * `page_view` fires on every route change and dominates event volume, so the
 * platform can ask clients to keep only a share of it. The decision is taken at
 * CAPTURE time — sampled-out events never enter the client queue, the beacon
 * payload or the desktop SQLite buffer, so sampling saves client storage,
 * bandwidth and server rows alike.
 *
 * FAIL-OPEN: every rate defaults to 1.0 (keep everything) until the platform
 * config lands, and stays at 1.0 when the config is unreachable. Losing data is
 * the worse failure — a client that cannot reach the config endpoint is usually
 * a client whose events we most want. `page_view` fails open too: the first few
 * captures of a session may therefore be kept even on an install that the
 * config later samples out.
 *
 * STABILITY: once the config has landed, the keep/drop decision for an event
 * name is drawn once and reused for the rest of the session, so a sampled-out
 * install stays sampled out instead of flipping mid-session and producing a
 * half-recorded journey. Rates of exactly 0 or 1 are deterministic and are
 * never memoized, which keeps the decision cache tiny in practice; the cache is
 * additionally capped so a caller emitting unbounded event names cannot grow it.
 *
 * INSTALL VISIBILITY: `page_view` is in practice the only usage event most
 * installs ever emit, and install-level rollups (DAU/WAU/MAU, retention) are
 * built from those rows. Sampling it naively would therefore delete active
 * installs from those metrics, not just volume. So while a rate is genuinely
 * sampling (strictly between 0 and 1) the FIRST page view of a session always
 * lands and only the rest of the session obeys the draw: an active install
 * stays visible for the day while the long tail of route changes is still cut.
 * An explicit rate of 0 is an operator kill switch and keeps nothing.
 *
 * This module deliberately knows nothing about the install id: the draw is
 * per-session randomness, never a function of identity.
 */

import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../schema/profile/profile";

export const TELEMETRY_PAGE_VIEW_EVENT = "page_view";
export const TELEMETRY_CONFIG_PATH = "telemetry/config";

export interface ITelemetrySamplingRates {
	/** Share of `page_view` captures to keep, in `0..=1`. */
	pageView: number;
	/** Share of all other product events to keep, in `0..=1`. */
	event: number;
}

export interface ITelemetrySamplingConfig {
	sampling: ITelemetrySamplingRates;
	/** False when the platform discards usage events entirely. */
	enabled: boolean;
}

/** Fetches `GET /telemetry/config`; may reject or resolve nothing when unreachable. */
export type TelemetrySamplingFetcher = () => Promise<
	ITelemetrySamplingConfig | undefined | null
>;

const KEEP_EVERYTHING: ITelemetrySamplingRates = { pageView: 1, event: 1 };
const MAX_SAMPLED_EVENT_NAMES = 256;
const MAX_LOAD_ATTEMPTS = 3;

let rates: ITelemetrySamplingRates = { ...KEEP_EVERYTHING };
let samplingEnabled = true;
let resolved = false;
let attempts = 0;
let pendingLoad: Promise<void> | undefined;
let pageViewSeen = false;
const decisions = new Map<string, boolean>();

function clampRate(value: unknown): number {
	if (typeof value !== "number" || !Number.isFinite(value)) return 1;
	return Math.min(1, Math.max(0, value));
}

function applyConfig(config: ITelemetrySamplingConfig | undefined | null) {
	rates = {
		pageView: clampRate(config?.sampling?.pageView),
		event: clampRate(config?.sampling?.event),
	};
	samplingEnabled = config?.enabled !== false;
	decisions.clear();
	resolved = true;
}

async function loadConfig(fetcher: TelemetrySamplingFetcher) {
	try {
		applyConfig(await fetcher());
	} catch {
		// Unreachable config: stay unresolved so captures keep failing open. The
		// call is retryable because hosts commonly init before the profile (and
		// therefore the API transport) is ready; the attempt cap keeps a hostile
		// render loop from turning that into a request storm.
	}
	pendingLoad = undefined;
}

/**
 * Loads the platform sampling config once per session and caches it in module
 * scope. Safe to call repeatedly — concurrent calls share one request and later
 * calls are no-ops once the config landed. A failed attempt is retried by the
 * next call, at most `MAX_LOAD_ATTEMPTS` times per session.
 */
export function initTelemetrySampling(
	fetcher: TelemetrySamplingFetcher,
): Promise<void> {
	if (resolved) return Promise.resolve();
	if (pendingLoad) return pendingLoad;
	if (attempts >= MAX_LOAD_ATTEMPTS) return Promise.resolve();
	attempts++;
	pendingLoad = loadConfig(fetcher);
	return pendingLoad;
}

/**
 * The standard fetcher: `GET /telemetry/config` through the app's API state.
 * Rejects while no profile is available so the attempt stays retryable instead
 * of locking the session into the fail-open rates.
 */
export function createTelemetrySamplingFetcher(
	apiState: IApiState,
	getProfile: () => IProfile | undefined,
): TelemetrySamplingFetcher {
	return async () => {
		const profile = getProfile();
		if (!profile) throw new Error("Telemetry config requires a profile");
		return apiState.get<ITelemetrySamplingConfig>(
			profile,
			TELEMETRY_CONFIG_PATH,
		);
	};
}

/**
 * Whether an event of this name should be captured. Returns true while the
 * config is still loading and whenever it could not be fetched.
 *
 * Call exactly once per capture: it advances per-session state (the memoized
 * decision and the first-page-view exemption), so asking twice for the same
 * capture consumes two decisions.
 */
export function shouldSampleEvent(name: string): boolean {
	try {
		if (!resolved) return true;
		if (!samplingEnabled) return false;

		const isPageView = name === TELEMETRY_PAGE_VIEW_EVENT;
		const rate = isPageView ? rates.pageView : rates.event;
		if (rate >= 1) return true;
		if (rate <= 0) return false;

		if (isPageView && !pageViewSeen) {
			pageViewSeen = true;
			return true;
		}

		const decided = decisions.get(name);
		if (decided !== undefined) return decided;

		const keep = Math.random() < rate;
		if (decisions.size < MAX_SAMPLED_EVENT_NAMES) decisions.set(name, keep);
		return keep;
	} catch {
		return true;
	}
}

/**
 * Drops the cached config and every memoized decision. Used when consent is
 * revoked and re-granted, and by tests.
 */
export function resetTelemetrySampling() {
	rates = { ...KEEP_EVERYTHING };
	samplingEnabled = true;
	resolved = false;
	attempts = 0;
	pendingLoad = undefined;
	pageViewSeen = false;
	decisions.clear();
}
