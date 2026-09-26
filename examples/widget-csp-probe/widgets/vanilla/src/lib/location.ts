import type { ProbeExpectation } from "./report";

export type ExpectationInput = "auto" | ProbeExpectation;

export type DocumentKind = "baseline" | "grant" | "legacy" | "unknown";

/** Runtime slots as the web document URL carries them: slot path → origins */
export type RuntimeSlots = Record<string, string[]>;

export interface DocumentInfo {
	kind: DocumentKind;
	widgetId: string | null;
	/** Path with any grant replaced by `{grant}`, safe to report */
	label: string;
	/** True when served by the web API (`widget-sandbox` route) */
	web: boolean;
	/** The raw grant of a granted document; never report it */
	grant: string | null;
	/** Runtime component after `~` in a web grant segment */
	runtime: string | null;
}

export const BASELINE_GRANT = "0";
export const RUNTIME_SEPARATOR = "~";
export const NAVIGATION_MARKER = "flw-csp-probe";
export const HISTORY_BACK_STEP = "flw-csp-probe-step";
export const HISTORY_BACK_STEP_VALUE = "history-back";

const ENTRY_PATH =
	/\/widgets\/([a-z0-9-]+)\/index(?:\.([A-Za-z0-9_.~-]+))?\.html$/;

export function readDocumentInfo(
	url: URL = new URL(location.href),
): DocumentInfo {
	const web = url.pathname.includes("/widget-sandbox/");
	const match = ENTRY_PATH.exec(url.pathname);
	if (!match) {
		return {
			kind: "unknown",
			widgetId: null,
			label: url.pathname,
			web,
			grant: null,
			runtime: null,
		};
	}
	const [, widgetId = "", segment] = match;
	const [grant, runtime = null] =
		segment === undefined ? [undefined] : segment.split(RUNTIME_SEPARATOR, 2);
	const kind: DocumentKind =
		grant === undefined
			? "legacy"
			: grant === BASELINE_GRANT
				? "baseline"
				: "grant";
	const shown =
		kind === "grant"
			? `.{grant}${runtime === null ? "" : `${RUNTIME_SEPARATOR}{runtime}`}`
			: segment
				? `.${segment}`
				: "";
	return {
		kind,
		widgetId,
		label: `widgets/${widgetId}/index${shown}.html`,
		web,
		grant: kind === "grant" ? (grant ?? null) : null,
		runtime,
	};
}

export function resolveExpectation(
	input: ExpectationInput,
	info: DocumentInfo,
	preview: boolean,
): ProbeExpectation {
	if (input !== "auto") return input;
	if (info.kind !== "grant") return "baseline";
	return preview ? "preview" : "granted";
}

export function navigationMarker(
	url: URL = new URL(location.href),
): string | null {
	const params = new URLSearchParams(url.hash.slice(1));
	return params.get(NAVIGATION_MARKER);
}

export function isHistoryBackStep(url: URL = new URL(location.href)): boolean {
	return url.searchParams.get(HISTORY_BACK_STEP) === HISTORY_BACK_STEP_VALUE;
}

export function withMarker(target: URL, checkId: string): string {
	const marked = new URL(target.href);
	marked.hash = `${NAVIGATION_MARKER}=${encodeURIComponent(checkId)}`;
	return marked.href;
}

/** A grant that no host issued, shaped like the current platform's grants */
export function foreignGrant(info: DocumentInfo): string {
	return info.web
		? "eyJhbGciOiJFUzI1NiJ9.eyJwcm9iZSI6ImNzcCJ9.Zm9yZWlnbi1ncmFudA"
		: "c5b0".repeat(16);
}

/** base64url without padding of the canonical JSON, as the host builds it */
export function encodeRuntimeComponent(slots: RuntimeSlots): string {
	const canonical = JSON.stringify(
		Object.fromEntries(
			Object.keys(slots)
				.sort()
				.map((slot) => [slot, [...(slots[slot] ?? [])].sort()]),
		),
	);
	return btoa(canonical)
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/, "");
}

export function decodeRuntimeComponent(
	component: string | null,
): RuntimeSlots | null {
	if (component === null) return null;
	try {
		const base64 = component.replaceAll("-", "+").replaceAll("_", "/");
		const parsed: unknown = JSON.parse(atob(base64));
		if (typeof parsed !== "object" || parsed === null) return null;
		const slots: RuntimeSlots = {};
		for (const [slot, sources] of Object.entries(parsed)) {
			if (!Array.isArray(sources)) return null;
			slots[slot] = sources.filter(
				(source): source is string => typeof source === "string",
			);
		}
		return slots;
	} catch {
		return null;
	}
}
