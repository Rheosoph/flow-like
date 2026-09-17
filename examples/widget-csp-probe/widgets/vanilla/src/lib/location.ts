import type { ProbeExpectation } from "./report";

export type ExpectationInput = "auto" | ProbeExpectation;

export type DocumentKind = "baseline" | "grant" | "legacy" | "unknown";

export interface DocumentInfo {
	kind: DocumentKind;
	widgetId: string | null;
	/** Path with any grant replaced by `{grant}`, safe to report */
	label: string;
	/** True when served by the web API (`widget-sandbox` route) */
	web: boolean;
}

export const BASELINE_GRANT = "0";
export const NAVIGATION_MARKER = "flw-csp-probe";
export const HISTORY_BACK_STEP = "flw-csp-probe-step";
export const HISTORY_BACK_STEP_VALUE = "history-back";

const ENTRY_PATH =
	/\/widgets\/([a-z0-9-]+)\/index(?:\.([A-Za-z0-9_.-]+))?\.html$/;

export function readDocumentInfo(
	url: URL = new URL(location.href),
): DocumentInfo {
	const web = url.pathname.includes("/widget-sandbox/");
	const match = ENTRY_PATH.exec(url.pathname);
	if (!match) {
		return { kind: "unknown", widgetId: null, label: url.pathname, web };
	}
	const [, widgetId = "", grant] = match;
	const kind: DocumentKind =
		grant === undefined
			? "legacy"
			: grant === BASELINE_GRANT
				? "baseline"
				: "grant";
	const segment = kind === "grant" ? ".{grant}" : grant ? `.${grant}` : "";
	return {
		kind,
		widgetId,
		label: `widgets/${widgetId}/index${segment}.html`,
		web,
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
