const MAX_REPORTS_PER_DOCUMENT = 20;
const MAX_URI_LENGTH = 4096;
const DIRECTIVES = new Set([
	"script-src",
	"script-src-elem",
	"script-src-attr",
	"worker-src",
	"object-src",
	"base-uri",
]);

type ResourceKind =
	| "inline"
	| "eval"
	| "wasm-eval"
	| "same-origin"
	| "cross-origin"
	| "blob"
	| "data"
	| "other";

type CspReport = { directive: string; resource: ResourceKind };

// Keep the limit across provider remounts and consent changes in the same page.
const reportedByDocument = new WeakMap<Document, Set<string>>();

function resourceKind(blockedURI: unknown, origin: string): ResourceKind {
	if (typeof blockedURI !== "string" || blockedURI.length > MAX_URI_LENGTH)
		return "other";
	if (
		blockedURI === "inline" ||
		blockedURI === "eval" ||
		blockedURI === "wasm-eval" ||
		blockedURI === "blob" ||
		blockedURI === "data"
	)
		return blockedURI;
	try {
		const url = new URL(blockedURI);
		if (url.protocol === "blob:") return "blob";
		if (url.protocol === "data:") return "data";
		if (url.protocol === "https:" || url.protocol === "http:")
			return url.origin === origin ? "same-origin" : "cross-origin";
	} catch {
		// Browsers can report an empty URI or a non-URL resource identifier.
	}
	return "other";
}

/** Call only while usage telemetry is enabled. Raw CSP reports can contain secrets. */
export function installCspReporting(
	document: Document,
	capture: (report: CspReport) => void,
): () => void {
	let reported = reportedByDocument.get(document);
	if (!reported) {
		reported = new Set();
		reportedByDocument.set(document, reported);
	}
	const seen = reported;
	const origin = document.location.origin;
	const onViolation = (event: SecurityPolicyViolationEvent) => {
		if (
			event.disposition !== "report" ||
			!DIRECTIVES.has(event.effectiveDirective) ||
			seen.size >= MAX_REPORTS_PER_DOCUMENT
		)
			return;

		const report: CspReport = {
			directive: event.effectiveDirective,
			resource: resourceKind(event.blockedURI, origin),
		};
		const key = `${report.directive}:${report.resource}`;
		if (seen.has(key)) return;
		seen.add(key);
		try {
			capture(report);
		} catch {
			// Reporting must not affect the application or user-authored content.
		}
	};
	document.addEventListener("securitypolicyviolation", onViolation);
	return () =>
		document.removeEventListener("securitypolicyviolation", onViolation);
}
