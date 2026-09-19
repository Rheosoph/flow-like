/** `review` means the probe could not decide on its own: read `observed`. */
export type ProbeStatus = "pass" | "fail" | "review" | "skip";

/** What the host should have granted the document under test */
export type ProbeExpectation = "granted" | "preview" | "baseline";

/** `local` is the local-scheme run, whose verdict partly lives in the canary server log */
export type ProbePhase = "suite" | "local" | "navigation";

export interface ProbeCheck {
	status: ProbeStatus;
	expected: string;
	observed: string;
}

export interface ProbeSummary {
	pass: number;
	fail: number;
	review: number;
	skip: number;
}

export interface ProbeReport {
	/** Widget whose document produced the report */
	widgetId: string;
	phase: ProbePhase;
	expectation: ProbeExpectation;
	/** Document path with any grant replaced by `{grant}` */
	document: string;
	userAgent: string;
	/** True when no check failed; `review` entries still need a human */
	passed: boolean;
	/** Canary URL prefix of this run; the canary server log must show no request below it */
	canary?: string;
	summary: ProbeSummary;
	checks: Record<string, ProbeCheck>;
}

export type ProbeChecks = Record<string, ProbeCheck>;

const GRANT = "[0-9a-f]{64}|[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+";
const RUNTIME = "~[A-Za-z0-9_-]+";
const GRANT_IN_DOCUMENT = new RegExp(
	`(index\\.)(?:${GRANT})(${RUNTIME})?(\\.html)`,
	"g",
);
const GRANT_IN_FRAME = new RegExp(
	`(/frame/[a-z0-9-]+/)(?:${GRANT})(${RUNTIME})?`,
	"g",
);

const redactedSegment = (runtime: string | undefined) =>
	runtime ? "{grant}~{runtime}" : "{grant}";

/** Engine error messages can quote document URLs; grants must not reach flows */
export function redactGrants(text: string): string {
	return text
		.replace(
			GRANT_IN_DOCUMENT,
			(_match, prefix: string, runtime: string | undefined, suffix: string) =>
				`${prefix}${redactedSegment(runtime)}${suffix}`,
		)
		.replace(
			GRANT_IN_FRAME,
			(_match, prefix: string, runtime: string | undefined) =>
				`${prefix}${redactedSegment(runtime)}`,
		);
}

export function check(
	status: ProbeStatus,
	expected: string,
	observed: string,
): ProbeCheck {
	return { status, expected, observed: redactGrants(observed) };
}

export function summarize(checks: ProbeChecks): ProbeSummary {
	const summary: ProbeSummary = { pass: 0, fail: 0, review: 0, skip: 0 };
	for (const entry of Object.values(checks)) summary[entry.status] += 1;
	return summary;
}

export function createReport(
	fields: Omit<ProbeReport, "passed" | "summary" | "userAgent">,
): ProbeReport {
	const summary = summarize(fields.checks);
	return {
		...fields,
		userAgent: navigator.userAgent,
		passed: summary.fail === 0,
		summary,
	};
}

export function describeError(error: unknown): string {
	if (error instanceof Error) return `${error.name}: ${error.message}`;
	return String(error);
}

export function delay(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

export function withTimeout<T>(
	promise: Promise<T>,
	ms: number,
	label: string,
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(
			() => reject(new Error(`${label} did not settle within ${ms} ms`)),
			ms,
		);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error: unknown) => {
				clearTimeout(timer);
				reject(error);
			},
		);
	});
}
