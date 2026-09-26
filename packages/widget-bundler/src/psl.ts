// Mirror of the ICANN parts of packages/wasm/schema/src/widget_sources
// (index.rs, rules.rs), built from the generated PSL rules. Driven by
// packages/wasm/schema/tests/fixtures/widget_source_classification.json.

import { PUBLIC_SUFFIX_RULES } from "./generated/widget-source-data";

export const WILDCARD_PUBLIC_SUFFIX_MESSAGE =
	"wildcard base is a public suffix or spans public suffixes";

interface IcannIndex {
	normal: Set<string>;
	wildcard: Set<string>;
	exception: Set<string>;
	tlds: Set<string>;
	below: Set<string>;
}

let index: IcannIndex | null = null;

function labelStarts(host: string): number[] {
	const starts = [0];
	for (let position = 0; position < host.length; position++) {
		if (host[position] === ".") starts.push(position + 1);
	}
	return starts;
}

function lastLabel(domain: string): string {
	return domain.slice(domain.lastIndexOf(".") + 1);
}

function strictAncestors(domain: string): string[] {
	return labelStarts(domain)
		.slice(1)
		.map((start) => domain.slice(start));
}

/** Label count of the ICANN public suffix of `host`, implicit `*` included. */
function icannSuffixLabels(rules: IcannIndex, host: string): number {
	const starts = labelStarts(host);
	const count = starts.length;
	const exception = starts.findIndex((start) =>
		rules.exception.has(host.slice(start)),
	);
	if (exception >= 0) return count - exception - 1;
	const matched = starts.findIndex((start, position) => {
		const next = starts[position + 1];
		return (
			rules.normal.has(host.slice(start)) ||
			(next !== undefined && rules.wildcard.has(host.slice(next)))
		);
	});
	return matched >= 0 ? count - matched : 1;
}

function isSuffixIn(rules: IcannIndex, host: string): boolean {
	return icannSuffixLabels(rules, host) >= labelStarts(host).length;
}

function buildIndex(): IcannIndex {
	const rules: IcannIndex = {
		normal: new Set(),
		wildcard: new Set(),
		exception: new Set(),
		tlds: new Set(),
		below: new Set(),
	};
	const wildcardDomains: string[] = [];
	const normalDomains: string[] = [];
	for (const line of PUBLIC_SUFFIX_RULES.split("\n")) {
		if (!line.startsWith("i ")) continue;
		const rule = line.slice(2);
		if (rule.startsWith("*.")) {
			const domain = rule.slice(2);
			rules.wildcard.add(domain);
			wildcardDomains.push(domain);
			rules.tlds.add(lastLabel(domain));
		} else if (rule.startsWith("!")) {
			const domain = rule.slice(1);
			rules.exception.add(domain);
			rules.tlds.add(lastLabel(domain));
		} else {
			rules.normal.add(rule);
			normalDomains.push(rule);
			rules.tlds.add(lastLabel(rule));
		}
	}
	const candidates = [
		...wildcardDomains.flatMap((domain) => [
			domain,
			...strictAncestors(domain),
		]),
		...normalDomains.flatMap(strictAncestors),
	];
	for (const candidate of candidates) {
		if (!isSuffixIn(rules, candidate)) rules.below.add(candidate);
	}
	return rules;
}

function icann(): IcannIndex {
	index ??= buildIndex();
	return index;
}

/** Whether `host` is itself an ICANN public suffix (`co.uk`, `foo.ck`). */
export function isIcannSuffix(host: string): boolean {
	return isSuffixIn(icann(), host);
}

/** Whether `host` is a strict ancestor of an ICANN rule without being one (`kawasaki.jp`). */
export function isIcannBelow(host: string): boolean {
	return icann().below.has(host);
}

/** Whether `label` is the last label of an ICANN rule. */
export function isIcannTld(label: string): boolean {
	return icann().tlds.has(label);
}

/** Base of a leading-label wildcard source (`https://*.b.com` → `b.com`). */
export function wildcardSourceBase(source: string): string | null {
	const separator = source.indexOf("://");
	if (separator < 0) return null;
	const host = source.slice(separator + 3).split("/")[0] ?? "";
	return host.startsWith("*.") ? host.slice(2) : null;
}

/**
 * Mirrors `validate_wildcard_bases`: a wildcard whose base is an ICANN public
 * suffix or spans one is rejected (`wildcard-public-suffix`). Other sources
 * are ignored; the structural grammar reports their errors.
 */
export function validateWildcardBases(sources: Iterable<string>): string[] {
	const errors: string[] = [];
	for (const source of sources) {
		const base = wildcardSourceBase(source);
		if (base !== null && (isIcannSuffix(base) || isIcannBelow(base))) {
			errors.push(
				`Invalid csp source "${source}": ${WILDCARD_PUBLIC_SUFFIX_MESSAGE}`,
			);
		}
	}
	return errors;
}
