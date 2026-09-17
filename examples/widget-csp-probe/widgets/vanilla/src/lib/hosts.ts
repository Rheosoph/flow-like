import type { WidgetCsp } from "@flow-like/widget-sdk";
import { type ProbeCheck, check } from "./report";

export interface ProbeEndpoint {
	origin: string;
	/** Any reachable URL; fetched with `mode: "no-cors"` */
	fetchUrl: string;
	/** A real image, so an unexpected load is unambiguous */
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

/** Loaded into a nested frame, which `frame-src 'none'` must refuse */
export const NESTED_FRAME_URL = "https://httpbin.org/html";

const EXPECTED_DECLARATION: Required<WidgetCsp> = {
	connectSrc: [GRANTED_HOST.origin, CONNECT_ONLY_HOST.origin],
	imgSrc: [GRANTED_HOST.origin],
	fontSrc: [],
	mediaSrc: [],
	styleSrc: [],
};

function sameSources(actual: readonly string[], expected: readonly string[]) {
	const left = [...actual].sort();
	const right = [...expected].sort();
	return (
		left.length === right.length &&
		left.every((source, index) => source === right[index])
	);
}

/** The probe's expectations only hold while `widget.config.ts` declares exactly these hosts */
export function checkDeclaration(csp: WidgetCsp | undefined): ProbeCheck {
	const expected = `connectSrc ${EXPECTED_DECLARATION.connectSrc.join(", ")}; imgSrc ${EXPECTED_DECLARATION.imgSrc.join(", ")}`;
	const drift = (Object.keys(EXPECTED_DECLARATION) as (keyof WidgetCsp)[])
		.filter(
			(directive) =>
				!sameSources(csp?.[directive] ?? [], EXPECTED_DECLARATION[directive]),
		)
		.map(
			(directive) => `${directive} is [${(csp?.[directive] ?? []).join(", ")}]`,
		);
	return drift.length === 0
		? check("pass", expected, "widget.config.ts matches src/lib/hosts.ts")
		: check(
				"fail",
				expected,
				`update src/lib/hosts.ts or widget.config.ts: ${drift.join("; ")}`,
			);
}
