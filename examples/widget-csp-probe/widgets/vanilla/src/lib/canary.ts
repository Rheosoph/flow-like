/**
 * The canary is a server whose request log the tester can read. Local-scheme
 * checks point absolute URLs inside `data:` and `blob:` media, documents and
 * artwork at it; a policy that holds leaves no entry in its log.
 */
export interface CanaryRun {
	origin: string;
	/** Unique per run: every URL a row may trigger starts with it */
	prefix: string;
}

export type CanarySetup =
	| { kind: "ready"; run: CanaryRun }
	| { kind: "missing" | "invalid"; reason: string };

const CANARY_HINT =
	"set the canaryUrl input to an https URL whose request log you can read";

function runId(): string {
	const random = crypto.getRandomValues(new Uint32Array(1))[0] ?? 0;
	return `${Date.now().toString(36)}${random.toString(36)}`;
}

export function canarySetup(input: string | undefined): CanarySetup {
	const value = input?.trim() ?? "";
	if (value === "") return { kind: "missing", reason: CANARY_HINT };
	let url: URL;
	try {
		url = new URL(value);
	} catch {
		return { kind: "invalid", reason: `canaryUrl ${value} is not a URL` };
	}
	if (url.protocol !== "https:") {
		return {
			kind: "invalid",
			reason: `canaryUrl must use https, so mixed-content blocking cannot hide a CSP failure (got ${url.protocol})`,
		};
	}
	const base = `${url.origin}${url.pathname.replace(/\/+$/, "")}`;
	return {
		kind: "ready",
		run: { origin: url.origin, prefix: `${base}/flw-csp-probe-${runId()}` },
	};
}

export function canaryUrl(run: CanaryRun, rowId: string, file: string): string {
	return `${run.prefix}/${rowId}/${file}`;
}

export function canaryExpectation(run: CanaryRun, rowId: string): string {
	return `no request under ${run.prefix}/${rowId}/ in the canary log`;
}
