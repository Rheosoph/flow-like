import type {
	WidgetCspDirective,
	WidgetCspPurpose,
} from "@flow-like/widget-sdk";
import { sourceCovers } from "./policy";
import { type ProbeCheck, check } from "./report";

export interface ProbeEndpoint {
	origin: string;
	/** Any reachable URL; fetched with `mode: "no-cors"` */
	fetchUrl: string;
	/** A real image served without a redirect, so an unexpected load is unambiguous */
	imageUrl: string;
}

/** Declared in `connectSrc` and `imgSrc` */
export const GRANTED_HOST: ProbeEndpoint = {
	origin: "https://httpbin.org",
	fetchUrl: "https://httpbin.org/get",
	imageUrl: "https://httpbin.org/image/png",
};

/** Declared in `connectSrc` only, so its images must stay blocked */
export const CONNECT_ONLY_HOST: ProbeEndpoint = {
	origin: "https://httpbingo.org",
	fetchUrl: "https://httpbingo.org/get",
	imageUrl: "https://httpbingo.org/image/png",
};

/** Declared nowhere */
export const UNDECLARED_HOST: ProbeEndpoint = {
	origin: "https://www.w3.org",
	fetchUrl: "https://www.w3.org/",
	imageUrl: "https://www.w3.org/Icons/w3c_home.png",
};

/** Declared in `connectSrc` and `imgSrc`; covers subdomains, never the apex */
export const WILDCARD_SOURCE = "https://*.wikipedia.org";

export const WILDCARD_SUBDOMAIN: ProbeEndpoint = {
	origin: "https://www.wikipedia.org",
	fetchUrl: "https://www.wikipedia.org/static/favicon/wikipedia.ico",
	imageUrl: "https://www.wikipedia.org/static/apple-touch/wikipedia.png",
};

/** The wildcard base itself: CSP3 matching must refuse it */
export const WILDCARD_APEX: ProbeEndpoint = {
	origin: "https://wikipedia.org",
	fetchUrl: "https://wikipedia.org/static/favicon/wikipedia.ico",
	imageUrl: "https://wikipedia.org/static/apple-touch/wikipedia.png",
};

/** Loaded into a nested frame, which `frame-src 'none'` must refuse */
export const NESTED_FRAME_URL = "https://httpbin.org/html";

/** Network inputs of the runtime purpose; the viewer approves the first and refuses the second */
export const RUNTIME_INPUTS = [
	"runtimeApprovedUrl",
	"runtimeRefusedUrl",
] as const;
export type RuntimeInput = (typeof RUNTIME_INPUTS)[number];

const DIRECTIVES: readonly WidgetCspDirective[] = [
	"connectSrc",
	"imgSrc",
	"fontSrc",
	"mediaSrc",
	"styleSrc",
];

const RUNTIME_DIRECTIVES: readonly WidgetCspDirective[] = [
	"connectSrc",
	"imgSrc",
];

const EXPECTED_SOURCES: Record<WidgetCspDirective, readonly string[]> = {
	connectSrc: [GRANTED_HOST.origin, CONNECT_ONLY_HOST.origin, WILDCARD_SOURCE],
	imgSrc: [GRANTED_HOST.origin, WILDCARD_SOURCE],
	fontSrc: [],
	mediaSrc: [],
	styleSrc: [],
};

const EXPECTED_INPUTS = RUNTIME_INPUTS.map((path) =>
	inputLabel(path, RUNTIME_DIRECTIVES),
);

function inputLabel(
	path: string,
	directives: readonly WidgetCspDirective[],
): string {
	return `${path} (${[...directives].sort().join(", ")})`;
}

function sorted(values: readonly string[]): string[] {
	return [...values].sort();
}

function sameList(actual: readonly string[], expected: readonly string[]) {
	const left = sorted(actual);
	const right = sorted(expected);
	return (
		left.length === right.length &&
		left.every((value, index) => value === right[index])
	);
}

function declaredSources(
	csp: readonly WidgetCspPurpose[],
	directive: WidgetCspDirective,
): string[] {
	return csp.flatMap((purpose) => purpose[directive] ?? []);
}

function declaredInputs(csp: readonly WidgetCspPurpose[]): string[] {
	return csp.flatMap((purpose) =>
		(purpose.inputs ?? []).map((input) =>
			inputLabel(input.path, input.directives),
		),
	);
}

/** Static sources of every directive that cover `origin` */
export function declaredDirectivesFor(
	csp: readonly WidgetCspPurpose[] | undefined,
	origin: string,
): WidgetCspDirective[] {
	return DIRECTIVES.filter((directive) =>
		declaredSources(csp ?? [], directive).some((source) =>
			sourceCovers(source, origin),
		),
	);
}

/** The probe's expectations only hold while `widget.config.ts` declares exactly these sources and inputs */
export function checkDeclaration(
	csp: readonly WidgetCspPurpose[] | undefined,
): ProbeCheck {
	const purposes = csp ?? [];
	const expected = [
		...DIRECTIVES.filter(
			(directive) => EXPECTED_SOURCES[directive].length > 0,
		).map(
			(directive) => `${directive} ${EXPECTED_SOURCES[directive].join(", ")}`,
		),
		`inputs ${EXPECTED_INPUTS.join(", ")}`,
	].join("; ");
	const drift = DIRECTIVES.filter(
		(directive) =>
			!sameList(
				declaredSources(purposes, directive),
				EXPECTED_SOURCES[directive],
			),
	).map(
		(directive) =>
			`${directive} is [${declaredSources(purposes, directive).join(", ")}]`,
	);
	const inputs = declaredInputs(purposes);
	if (!sameList(inputs, EXPECTED_INPUTS)) {
		drift.push(`inputs are [${inputs.join(", ")}]`);
	}
	return drift.length === 0
		? check("pass", expected, "widget.config.ts matches src/lib/hosts.ts")
		: check(
				"fail",
				expected,
				`update src/lib/hosts.ts or widget.config.ts: ${drift.join("; ")}`,
			);
}
