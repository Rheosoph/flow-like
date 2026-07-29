export type TelemetryConsent = "granted" | "denied" | undefined;

const CONSENT_KEY = "flow-like:telemetry";
const CRASH_REPORTS_KEY = "flow-like:crash-reports";
const ANON_ID_KEY = "flow-like:telemetry-anon-id";
const CONSENT_EVENT = "flow-like:telemetry-consent";

export function getTelemetryConsent(): TelemetryConsent {
	if (typeof window === "undefined") return undefined;
	const value = window.localStorage.getItem(CONSENT_KEY);
	return value === "granted" || value === "denied" ? value : undefined;
}

/**
 * Crash and error reporting is a separate consent from usage telemetry and
 * defaults on: an absent key means enabled, only "denied" turns it off.
 */
export function getCrashReportsEnabled(): boolean {
	if (typeof window === "undefined") return false;
	return window.localStorage.getItem(CRASH_REPORTS_KEY) !== "denied";
}

/**
 * The random install id is shared by crash and usage telemetry. It is minted on
 * first need and only dropped once both consents are off, so the next opt-in
 * starts from a fresh identity.
 */
function syncAnonId(): string | undefined {
	if (typeof window === "undefined") return undefined;
	if (getTelemetryConsent() !== "granted" && !getCrashReportsEnabled()) {
		window.localStorage.removeItem(ANON_ID_KEY);
		return undefined;
	}
	const existing = window.localStorage.getItem(ANON_ID_KEY);
	if (existing) return existing;
	const minted = crypto.randomUUID();
	window.localStorage.setItem(ANON_ID_KEY, minted);
	return minted;
}

export function getTelemetryAnonId(): string | undefined {
	return syncAnonId();
}

export function setTelemetryConsent(enabled: boolean): void {
	if (typeof window === "undefined") return;
	window.localStorage.setItem(CONSENT_KEY, enabled ? "granted" : "denied");
	syncAnonId();
	window.dispatchEvent(new Event(CONSENT_EVENT));
}

export function setCrashReportsEnabled(enabled: boolean): void {
	if (typeof window === "undefined") return;
	window.localStorage.setItem(
		CRASH_REPORTS_KEY,
		enabled ? "granted" : "denied",
	);
	syncAnonId();
	window.dispatchEvent(new Event(CONSENT_EVENT));
}

export function onTelemetryConsentChange(listener: () => void): () => void {
	window.addEventListener(CONSENT_EVENT, listener);
	return () => window.removeEventListener(CONSENT_EVENT, listener);
}
