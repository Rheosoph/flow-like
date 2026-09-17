import { GRANTED_HOST } from "./hosts";
import { type ViolationWatch, probeFetch } from "./network";
import {
	type ProbeCheck,
	type ProbeExpectation,
	check,
	describeError,
	withTimeout,
} from "./report";

const FRAME_WALK_LIMIT = 512;
const FRAME_WALK_DEPTH = 8;
const IPC_TIMEOUT_MS = 3_000;
const IPC_COMMAND = "plugin:app|version";
const IPC_PROTOCOL_URLS = [
	"ipc://localhost/plugin%3Aapp%7Cversion",
	"http://ipc.localhost/plugin%3Aapp%7Cversion",
];

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function globalValue(path: readonly string[]): unknown {
	let current: unknown = globalThis;
	for (const key of path) {
		if (!isRecord(current)) return undefined;
		try {
			current = current[key];
		} catch {
			return undefined;
		}
	}
	return current;
}

function isHosted(): boolean {
	try {
		return window.top !== window;
	} catch {
		return true;
	}
}

function hostedOnly(expected: string): ProbeCheck {
	return check("skip", expected, "not running inside a host frame");
}

export function checkOpaqueOrigin(): ProbeCheck {
	const expected = "opaque origin (serialized as null)";
	return self.origin === "null"
		? check("pass", expected, "origin is null")
		: check("fail", expected, `origin is ${self.origin}`);
}

export function checkNoReferrer(): ProbeCheck {
	const expected = "empty document.referrer";
	return document.referrer === ""
		? check("pass", expected, "referrer is empty")
		: check("fail", expected, `referrer is ${document.referrer}`);
}

function directiveSources(policy: string, directive: string): string[] | null {
	for (const part of policy.split(";")) {
		const [name, ...sources] = part.trim().split(/\s+/);
		if (name === directive) return sources;
	}
	return null;
}

/** The server replaces every packed CSP meta with one copy of the header policy */
export function checkCspMeta(expectation: ProbeExpectation): ProbeCheck {
	const metas = [...document.querySelectorAll("meta[http-equiv]")].filter(
		(meta) =>
			meta.getAttribute("http-equiv")?.toLowerCase() ===
			"content-security-policy",
	);
	const wantsHost = expectation === "granted";
	const expected = `one CSP meta without 'self', connect-src ${wantsHost ? "lists" : "omits"} ${GRANTED_HOST.origin}`;
	if (metas.length !== 1) {
		return check("fail", expected, `${metas.length} CSP meta tags`);
	}
	const policy = metas[0]?.getAttribute("content") ?? "";
	const problems: string[] = [];
	if (policy.includes("'self'")) problems.push("policy contains 'self'");
	const connect = directiveSources(policy, "connect-src");
	if (connect === null) {
		problems.push("no connect-src directive");
	} else if (connect.includes(GRANTED_HOST.origin) !== wantsHost) {
		problems.push(`connect-src is ${connect.join(" ")}`);
	}
	return problems.length === 0
		? check("pass", expected, "server-injected meta matches the expectation")
		: check("fail", expected, problems.join("; "));
}

export function checkParentChildren(): ProbeCheck {
	const expected =
		"parent.length === 0 (child frame sits in a closed shadow root)";
	if (!isHosted()) return hostedOnly(expected);
	try {
		const count = window.parent.length;
		return count === 0
			? check("pass", expected, "parent exposes no child frames")
			: check("fail", expected, `parent exposes ${count} child frame(s)`);
	} catch (error) {
		return check("review", expected, describeError(error));
	}
}

interface FrameWalk {
	visited: number;
	foundSelf: boolean;
	/** Frames below `top` that expose child frames */
	withChildren: number;
}

/** Breadth-first walk of `top.frames`, the way a widget would look for other widgets */
function walkFrames(): FrameWalk {
	const queue: [Window, number][] = [[window.top as Window, 0]];
	const walk: FrameWalk = { visited: 0, foundSelf: false, withChildren: 0 };
	while (queue.length > 0 && walk.visited < FRAME_WALK_LIMIT) {
		const [frame, depth] = queue.shift() as [Window, number];
		walk.visited += 1;
		if (frame === window) walk.foundSelf = true;
		let length = 0;
		try {
			length = frame.length;
		} catch {
			continue;
		}
		if (depth > 0 && length > 0) walk.withChildren += 1;
		if (depth >= FRAME_WALK_DEPTH) continue;
		for (let index = 0; index < length; index += 1) {
			try {
				const child = frame.frames[index];
				if (child) queue.push([child, depth + 1]);
			} catch {
				// Cross-origin indexed access can throw on some engines.
			}
		}
	}
	return walk;
}

export function checkFrameTree(): Record<string, ProbeCheck> {
	const selfExpected = "this document is not reachable through top.frames";
	const childrenExpected =
		"no frame below top exposes child frames (every wrapper reports length 0)";
	if (!isHosted()) {
		return {
			"frames.selfUnreachable": hostedOnly(selfExpected),
			"frames.wrappersHaveNoChildren": hostedOnly(childrenExpected),
		};
	}
	const walk = walkFrames();
	const observed = `visited ${walk.visited} frame(s), ${walk.withChildren} below top with children`;
	return {
		"frames.selfUnreachable": walk.foundSelf
			? check("fail", selfExpected, `found this document; ${observed}`)
			: check("pass", selfExpected, observed),
		"frames.wrappersHaveNoChildren":
			walk.withChildren === 0
				? check("pass", childrenExpected, observed)
				: check(
						"review",
						childrenExpected,
						`${observed}; a failure when every such frame is a widget wrapper, expected for page previews and embedded sites`,
					),
	};
}

export async function checkTauriInternals(): Promise<ProbeCheck> {
	const expected = "__TAURI_INTERNALS__ absent or unusable";
	const internals = globalValue(["__TAURI_INTERNALS__"]);
	if (internals === undefined) {
		return check("pass", expected, "not exposed");
	}
	const invoke = isRecord(internals) ? internals.invoke : undefined;
	if (typeof invoke !== "function") {
		return check("review", expected, "exposed without an invoke function");
	}
	try {
		const value = await withTimeout(
			Promise.resolve(invoke.call(internals, IPC_COMMAND)),
			IPC_TIMEOUT_MS,
			`invoke("${IPC_COMMAND}")`,
		);
		return check(
			"fail",
			expected,
			`invoke("${IPC_COMMAND}") resolved with ${JSON.stringify(value)}`,
		);
	} catch (error) {
		return check(
			"review",
			expected,
			`exposed; invoke("${IPC_COMMAND}") rejected: ${describeError(error)}`,
		);
	}
}

export function checkWebkitMessageHandler(): ProbeCheck {
	const expected = "webkit.messageHandlers.ipc absent";
	return globalValue(["webkit", "messageHandlers", "ipc"]) === undefined
		? check("pass", expected, "not exposed")
		: check(
				"review",
				expected,
				"exposed; confirm the native handler drops messages from subframes",
			);
}

export function checkWebView2Bridge(): ProbeCheck {
	const expected = "chrome.webview.postMessage absent";
	return typeof globalValue(["chrome", "webview", "postMessage"]) === "function"
		? check(
				"review",
				expected,
				"exposed; confirm the host ignores web messages from subframes",
			)
		: check("pass", expected, "not exposed");
}

export async function checkIpcProtocol(
	violations: ViolationWatch,
): Promise<ProbeCheck> {
	const expected = "custom IPC endpoints unreachable";
	const outcomes = await Promise.all(
		IPC_PROTOCOL_URLS.map(async (url) => ({
			url,
			outcome: await probeFetch(url, violations, {
				method: "POST",
				body: "{}",
			}),
		})),
	);
	const reached = outcomes.filter(({ outcome }) => outcome.reached);
	if (reached.length > 0) {
		return check(
			"fail",
			expected,
			`reached ${reached.map(({ url }) => url).join(", ")}`,
		);
	}
	return check(
		"pass",
		expected,
		outcomes
			.map(
				({ url, outcome }) =>
					`${new URL(url).protocol} ${outcome.violation ? `blocked by ${outcome.violation.directive}` : (outcome.error ?? "failed")}`,
			)
			.join("; "),
	);
}
