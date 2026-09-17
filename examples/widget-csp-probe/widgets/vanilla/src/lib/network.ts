import {
	type ProbeCheck,
	check,
	delay,
	describeError,
	withTimeout,
} from "./report";

const REQUEST_TIMEOUT_MS = 8_000;
const WORKER_TIMEOUT_MS = 20_000;
const FRAME_WAIT_MS = 3_000;
/** Violation events are queued as tasks and can trail the failed request */
const VIOLATION_SETTLE_MS = 150;

export interface ViolationRecord {
	blockedURI: string;
	directive: string;
}

export interface ViolationWatch {
	forUrl(url: string, directive: string): ViolationRecord | undefined;
	forDirective(...directives: string[]): ViolationRecord | undefined;
	dispose(): void;
}

export interface RequestOutcome {
	reached: boolean;
	error?: string;
	violation?: ViolationRecord;
}

function endpointKey(url: string): string | null {
	try {
		const parsed = new URL(url);
		return `${parsed.protocol}//${parsed.host}`;
	} catch {
		return null;
	}
}

/** Engines report either the full URL or only the origin of a cross-origin block */
export function sameEndpoint(blockedURI: string, url: string): boolean {
	if (blockedURI === url) return true;
	const key = endpointKey(url);
	return key !== null && endpointKey(blockedURI) === key;
}

export function watchViolations(target: EventTarget): ViolationWatch {
	const records: ViolationRecord[] = [];
	const listener = (event: Event) => {
		const violation = event as SecurityPolicyViolationEvent;
		records.push({
			blockedURI: violation.blockedURI,
			directive: violation.effectiveDirective || violation.violatedDirective,
		});
	};
	target.addEventListener("securitypolicyviolation", listener);
	return {
		forUrl(url, directive) {
			return records.find(
				(record) =>
					record.directive.startsWith(directive) &&
					sameEndpoint(record.blockedURI, url),
			);
		},
		forDirective(...directives) {
			return records.find((record) =>
				directives.some((directive) => record.directive.startsWith(directive)),
			);
		},
		dispose() {
			target.removeEventListener("securitypolicyviolation", listener);
		},
	};
}

export function evaluateOutcome(
	expectAllowed: boolean,
	outcome: RequestOutcome,
	reachedText: string,
): ProbeCheck {
	const expected = expectAllowed ? "allowed" : "blocked by CSP";
	if (outcome.reached) {
		return check(expectAllowed ? "pass" : "fail", expected, reachedText);
	}
	if (outcome.violation) {
		return check(
			expectAllowed ? "fail" : "pass",
			expected,
			`blocked by ${outcome.violation.directive}`,
		);
	}
	const failure = `failed without a CSP violation event (${outcome.error ?? "no error"})`;
	return check(
		"review",
		expected,
		expectAllowed
			? `${failure}; make sure the host is reachable from this device`
			: `${failure}; some engines block without dispatching the event (WebKit in workers), so look for a CSP message in the console`,
	);
}

function cacheBusted(url: string): string {
	const separator = url.includes("?") ? "&" : "?";
	return `${url}${separator}flw-csp-probe=${Date.now().toString(36)}`;
}

export async function probeFetch(
	url: string,
	violations: ViolationWatch,
	init: RequestInit = {},
): Promise<RequestOutcome> {
	const controller = new AbortController();
	const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
	try {
		await fetch(cacheBusted(url), {
			mode: "no-cors",
			cache: "no-store",
			credentials: "omit",
			referrerPolicy: "no-referrer",
			signal: controller.signal,
			...init,
		});
		return { reached: true };
	} catch (error) {
		await delay(VIOLATION_SETTLE_MS);
		return {
			reached: false,
			error: describeError(error),
			violation: violations.forUrl(url, "connect-src"),
		};
	} finally {
		clearTimeout(timer);
	}
}

export function probeImage(
	url: string,
	violations: ViolationWatch,
): Promise<RequestOutcome> {
	return new Promise((resolve) => {
		const image = new Image();
		const timer = setTimeout(
			() => finish(false, `no load event within ${REQUEST_TIMEOUT_MS} ms`),
			REQUEST_TIMEOUT_MS,
		);
		function finish(reached: boolean, error?: string) {
			clearTimeout(timer);
			image.onload = null;
			image.onerror = null;
			if (reached) {
				resolve({ reached: true });
				return;
			}
			delay(VIOLATION_SETTLE_MS).then(() =>
				resolve({
					reached: false,
					error,
					violation: violations.forUrl(url, "img-src"),
				}),
			);
		}
		image.onload = () => finish(true);
		image.onerror = () => finish(false, "image error event");
		image.referrerPolicy = "no-referrer";
		image.src = cacheBusted(url);
	});
}

const WORKER_SOURCE = `
const violations = [];
self.addEventListener("securitypolicyviolation", (event) => {
	violations.push({ blockedURI: event.blockedURI, directive: event.effectiveDirective || event.violatedDirective });
});
self.onmessage = async (event) => {
	const results = {};
	for (const [key, url] of Object.entries(event.data)) {
		const controller = new AbortController();
		const timer = setTimeout(() => controller.abort(), ${REQUEST_TIMEOUT_MS});
		try {
			await fetch(url, { mode: "no-cors", cache: "no-store", credentials: "omit", signal: controller.signal });
			results[key] = { reached: true };
		} catch (error) {
			results[key] = { reached: false, error: String(error) };
		} finally {
			clearTimeout(timer);
		}
	}
	await new Promise((resolve) => setTimeout(resolve, ${VIOLATION_SETTLE_MS}));
	self.postMessage({ results, violations });
};
`;

export interface WorkerProbe {
	start: RequestOutcome;
	results: Record<string, RequestOutcome>;
}

interface WorkerReply {
	results: Record<string, { reached: boolean; error?: string }>;
	violations: ViolationRecord[];
}

function workerOutcomes(
	urls: Record<string, string>,
	reply: WorkerReply,
): Record<string, RequestOutcome> {
	const outcomes: Record<string, RequestOutcome> = {};
	for (const [key, url] of Object.entries(urls)) {
		const result = reply.results[key] ?? {
			reached: false,
			error: "worker returned no result",
		};
		outcomes[key] = result.reached
			? { reached: true }
			: {
					reached: false,
					error: result.error,
					violation: reply.violations.find(
						(record) =>
							record.directive.startsWith("connect-src") &&
							sameEndpoint(record.blockedURI, url),
					),
				};
	}
	return outcomes;
}

/** Starts a blob worker (needs `workers`) and fetches each URL from inside it */
export async function probeWorker(
	urls: Record<string, string>,
	violations: ViolationWatch,
): Promise<WorkerProbe> {
	const blobUrl = URL.createObjectURL(
		new Blob([WORKER_SOURCE], { type: "text/javascript" }),
	);
	const notStarted = async (error: string): Promise<WorkerProbe> => {
		await delay(VIOLATION_SETTLE_MS);
		return {
			start: {
				reached: false,
				error,
				violation: violations.forDirective("worker-src", "script-src"),
			},
			results: {},
		};
	};

	let worker: Worker;
	try {
		worker = new Worker(blobUrl);
	} catch (error) {
		URL.revokeObjectURL(blobUrl);
		return notStarted(describeError(error));
	}

	const reply = new Promise<WorkerReply>((resolve, reject) => {
		worker.onmessage = (event: MessageEvent<WorkerReply>) =>
			resolve(event.data);
		worker.onerror = (event) => {
			event.preventDefault();
			reject(new Error(event.message || "worker error event"));
		};
	});
	worker.postMessage(urls);

	try {
		const data = await withTimeout(reply, WORKER_TIMEOUT_MS, "Worker");
		return { start: { reached: true }, results: workerOutcomes(urls, data) };
	} catch (error) {
		return notStarted(describeError(error));
	} finally {
		worker.terminate();
		URL.revokeObjectURL(blobUrl);
	}
}

/** Leaves the frame in `slot` so a tester can see whether the page rendered */
export async function probeNestedFrame(
	url: string,
	slot: HTMLElement,
	violations: ViolationWatch,
): Promise<ProbeCheck> {
	slot.replaceChildren();
	const frame = document.createElement("iframe");
	frame.title = "Nested frame probe";
	frame.referrerPolicy = "no-referrer";
	frame.src = url;
	slot.append(frame);
	await delay(FRAME_WAIT_MS);
	const violation = violations.forDirective("frame-src", "child-src");
	const expected = "blocked by frame-src 'none'";
	return violation
		? check("pass", expected, `blocked by ${violation.directive}`)
		: check(
				"review",
				expected,
				"no frame-src violation event; the frame under the results shows whether the page loaded",
			);
}
