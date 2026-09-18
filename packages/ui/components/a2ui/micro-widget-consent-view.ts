import {
	WIDGET_SOURCE_KIND_UNCLASSIFIED,
	type WidgetBannerLead,
	type WidgetConsentTitle,
} from "./micro-widget-network-copy";
import {
	type MicroWidgetCapability,
	WIDGET_CSP_KEYS,
	type WidgetCspKey,
	type WidgetNetworkSource,
	type WidgetRuntimeSourceEntry,
	type WidgetSourceLevel,
	maxWidgetSourceLevel,
	policyCapabilities,
	policySourceEntries,
	widgetSourceLevelRank,
} from "./micro-widget-policy";
import type { MicroWidgetConsentPrompt } from "./use-micro-widget-grant";

/**
 * Presentation of a frozen consent prompt (§14.5.2): purpose cards, host
 * chips, the aggregated banner lead, title and tone. Consent itself is
 * decided by the grant controller; this only groups what it asks for.
 */

/** Dialog tone of each capability (§14.3.4). */
export const WIDGET_CAPABILITY_TONE: Readonly<
	Record<MicroWidgetCapability, WidgetSourceLevel>
> = {
	workers: "known",
	wasm: "known",
	media: "known",
	microphone: "shared",
	downloads: "broad",
};

/** Capabilities that keep a full row; the rest share "Also asks for". */
export const WIDGET_FULL_ROW_CAPABILITIES: readonly MicroWidgetCapability[] = [
	"microphone",
	"downloads",
];

/** Declared chips shown on a card before "+N more". */
export const WIDGET_CONSENT_VISIBLE_CHIPS = 4;
/** `known` cards shown before the rest collapse into one line. */
export const WIDGET_CONSENT_VISIBLE_KNOWN_CARDS = 2;

export interface WidgetConsentSource extends WidgetNetworkSource {
	purpose: number;
	runtime: boolean;
	/** Asked for in this dialog: the uncovered declared part or a pending runtime source. */
	shown: boolean;
	/** Counts toward title, banner and tone: shown and not an unchecked runtime source. */
	included: boolean;
	isNew: boolean;
	raised: boolean;
}

export interface WidgetConsentChip {
	key: string;
	/** Host without the leading `*.`. */
	host: string;
	wildcard: boolean;
	emphasis: string;
	runtime: boolean;
	platformStorage: boolean;
	level: WidgetSourceLevel;
	isNew: boolean;
	raised: boolean;
	included: boolean;
}

export type WidgetHostNote =
	| { kind: "risk"; source: WidgetConsentSource }
	| { kind: "service"; source: WidgetConsentSource }
	| { kind: "runtime" };

export interface WidgetPurposeView {
	/** Declaration order of the purpose. */
	index: number;
	/** Publisher text; null for the unclassified fallback card. */
	reason: string | null;
	level: WidgetSourceLevel;
	directives: WidgetCspKey[];
	declared: WidgetConsentChip[];
	runtime: WidgetConsentChip[];
	/** Input paths the runtime chips arrived through. */
	inputs: string[];
	providers: string[];
	notes: WidgetHostNote[];
	/** Declared sources first, then runtime sources. */
	sources: WidgetConsentSource[];
	isNew: boolean;
	raised: boolean;
}

export interface WidgetConsentView {
	cards: WidgetPurposeView[];
	/** Every address of the frozen descriptor, for Details. */
	sources: WidgetConsentSource[];
	capabilities: MicroWidgetCapability[];
	networkLevel: WidgetSourceLevel | null;
	tone: WidgetSourceLevel;
	title: WidgetConsentTitle;
	lead: WidgetBannerLead;
	hasNetwork: boolean;
	/** Highest included source, first in card order. */
	top: WidgetConsentSource | null;
	/** Some of what the prompt asks for was allowed before. */
	expanded: boolean;
	alreadyAllowed: number;
	runtimeCount: number;
	hostCount: number;
	classified: boolean;
	wildcardUnsupported: boolean;
	loadsPassively: boolean;
	usesWebSockets: boolean;
}

function rank(level: WidgetSourceLevel): number {
	return widgetSourceLevelRank(level);
}

function sourceHost(source: string): string {
	const separator = source.indexOf("://");
	const rest = separator < 0 ? source : source.slice(separator + 3);
	const slash = rest.indexOf("/");
	return slash < 0 ? rest : rest.slice(0, slash);
}

function pairKey(directive: WidgetCspKey, source: string): string {
	return `${directive} ${source}`;
}

function pairsOf(entries: readonly WidgetRuntimeSourceEntry[]): Set<string> {
	return new Set(
		entries.map((entry) => pairKey(entry.directive, entry.source)),
	);
}

function distinct<T>(values: Iterable<T>): T[] {
	return [...new Set(values)];
}

interface PromptPurpose {
	reason: string | null;
	sources: WidgetNetworkSource[];
}

/** Without a classification every source is one broad, unclassified card (§14.2.6). */
function unclassifiedPurposes(
	prompt: MicroWidgetConsentPrompt,
): PromptPurpose[] {
	const policy = prompt.descriptor?.policy ?? prompt.subject.policy;
	const runtime = new Map(
		[...prompt.runtime.pending, ...prompt.runtime.allowed].map((entry) => [
			pairKey(entry.directive, entry.source),
			entry,
		]),
	);
	const grouped = new Map<string, WidgetNetworkSource>();
	for (const [directive, source] of policySourceEntries(policy)) {
		const entry = runtime.get(pairKey(directive, source));
		const key = `${entry ? "runtime" : "declared"} ${source}`;
		const existing = grouped.get(key);
		if (existing) {
			existing.directives.push(directive);
			continue;
		}
		const host = sourceHost(source);
		grouped.set(key, {
			source,
			directives: [directive],
			origin: entry ? "runtime" : "declared",
			...(entry?.slot ? { slot: entry.slot } : {}),
			kind: WIDGET_SOURCE_KIND_UNCLASSIFIED,
			level: "broad",
			host,
			emphasis: host.startsWith("*.") ? host.slice(2) : host,
		});
	}
	return grouped.size > 0
		? [{ reason: null, sources: [...grouped.values()] }]
		: [];
}

/** One chip per host (`https://h` and `wss://h` merge); a platform-storage scope keeps its path. */
function chipKey(source: WidgetNetworkSource): string {
	if (source.kind !== "platform-storage") return source.host;
	const separator = source.source.indexOf("://");
	return separator < 0 ? source.source : source.source.slice(separator + 3);
}

function mergeChips(
	sources: readonly WidgetConsentSource[],
): WidgetConsentChip[] {
	const chips = new Map<string, WidgetConsentChip>();
	for (const source of sources) {
		const key = chipKey(source);
		const existing = chips.get(key);
		if (existing) {
			if (rank(source.level) > rank(existing.level))
				existing.level = source.level;
			existing.isNew ||= source.isNew;
			existing.raised ||= source.raised;
			existing.included ||= source.included;
			continue;
		}
		const wildcard = source.host.startsWith("*.");
		chips.set(key, {
			key,
			host: wildcard ? source.host.slice(2) : source.host,
			wildcard,
			emphasis: source.emphasis,
			runtime: source.runtime,
			platformStorage: source.kind === "platform-storage",
			level: source.level,
			isNew: source.isNew,
			raised: source.raised,
			included: source.included,
		});
	}
	return [...chips.values()];
}

function maxLevel(
	sources: readonly WidgetConsentSource[],
): WidgetSourceLevel | null {
	return maxWidgetSourceLevel(sources.map((source) => source.level));
}

/**
 * Host copy of a card (§14.5.2 line 3): one Risk line per emphasis domain
 * above `known`, else one service line when the card sends data; runtime
 * chips add the runtime explanation, first when a runtime source is the top.
 */
function hostNotes(
	declared: readonly WidgetConsentSource[],
	runtime: readonly WidgetConsentSource[],
	directives: readonly WidgetCspKey[],
): WidgetHostNote[] {
	const notes: WidgetHostNote[] = [];
	const domains = new Set<string>();
	const ranked = [...declared].sort((a, b) => rank(b.level) - rank(a.level));
	for (const source of ranked) {
		if (source.level === "known" || domains.has(source.emphasis)) continue;
		domains.add(source.emphasis);
		notes.push({ kind: "risk", source });
	}
	if (notes.length === 0 && directives.includes("connectSrc")) {
		const service = declared.find((source) =>
			source.directives.includes("connectSrc"),
		);
		if (service) notes.push({ kind: "service", source: service });
	}
	if (runtime.length > 0) {
		const runtimeTop = maxLevel(runtime) ?? "broad";
		const declaredTop = maxLevel(declared);
		if (declaredTop === null || rank(runtimeTop) > rank(declaredTop))
			notes.unshift({ kind: "runtime" });
		else notes.push({ kind: "runtime" });
	}
	return notes;
}

function buildCard(
	index: number,
	reason: string | null,
	shown: WidgetConsentSource[],
): WidgetPurposeView | null {
	if (shown.length === 0) return null;
	const declared = shown.filter((source) => !source.runtime);
	const runtime = shown.filter((source) => source.runtime);
	const directives = WIDGET_CSP_KEYS.filter((key) =>
		shown.some((source) => source.directives.includes(key)),
	);
	return {
		index,
		reason,
		level: maxLevel(shown) ?? "broad",
		directives,
		declared: mergeChips(declared),
		runtime: mergeChips(runtime),
		inputs: distinct(
			runtime
				.map((source) => source.slot)
				.filter((slot): slot is string => typeof slot === "string"),
		),
		providers: distinct(
			shown
				.map((source) => source.provider)
				.filter((provider): provider is string => Boolean(provider)),
		),
		notes: hostNotes(declared, runtime, directives),
		sources: [...declared, ...runtime],
		isNew: shown.some((source) => source.isNew),
		raised: shown.some((source) => source.raised),
	};
}

function compareCards(a: WidgetPurposeView, b: WidgetPurposeView): number {
	return (
		rank(b.level) - rank(a.level) ||
		Number(b.isNew || b.raised) - Number(a.isNew || a.raised) ||
		a.index - b.index
	);
}

function bannerLead(
	top: WidgetConsentSource | null,
	included: readonly WidgetConsentSource[],
	classified: boolean,
): WidgetBannerLead {
	if (!top) return { kind: "capabilities" };
	if (!classified) return { kind: "unclassified", domain: top.emphasis };
	const provider = top.provider ?? top.emphasis;
	switch (top.level) {
		case "broad":
		case "shared":
			return { kind: top.level, provider };
		case "external":
			return {
				kind: "external",
				domain: top.emphasis,
				others: distinct(
					included
						.filter((source) => source.level === "external")
						.map((source) => source.emphasis)
						.filter((emphasis) => emphasis !== top.emphasis),
				).length,
			};
		case "known":
			return {
				kind: "known",
				provider,
				others: distinct(
					included
						.map((source) => source.provider ?? source.emphasis)
						.filter((name) => name !== provider),
				).length,
			};
	}
}

export function buildWidgetConsentView(
	prompt: MicroWidgetConsentPrompt,
): WidgetConsentView {
	const network = prompt.descriptor?.network;
	const classified = network !== undefined;
	const purposes: PromptPurpose[] = network
		? network.purposes.map((purpose) => ({
				reason: purpose.reason,
				sources: purpose.sources,
			}))
		: unclassifiedPurposes(prompt);
	const pending = pairsOf(prompt.runtime.pending);
	const declaredCovered = prompt.declared.covered;
	const requestedCapabilities = policyCapabilities(prompt.declared.policy);
	const newSources = new Set(prompt.newSources);
	const raisedSources = new Set(prompt.raisedSources);
	const allSources = purposes.flatMap((purpose) =>
		purpose.sources.map((source) => source.source),
	);
	const previouslyGranted =
		allSources.some((source) => !newSources.has(source)) ||
		requestedCapabilities.some(
			(name) => !prompt.newCapabilities.includes(name),
		);
	const marksNew = prompt.mode === "mount" && previouslyGranted;

	const sources: WidgetConsentSource[] = purposes.flatMap((purpose, index) =>
		purpose.sources.map((source) => {
			const runtime = source.origin === "runtime";
			const shown = runtime
				? source.directives.some((directive) =>
						pending.has(pairKey(directive, source.source)),
					)
				: !declaredCovered;
			return {
				...source,
				purpose: index,
				runtime,
				shown,
				included: shown && (!runtime || prompt.includeRuntime),
				isNew: marksNew && newSources.has(source.source),
				raised: raisedSources.has(source.source),
			};
		}),
	);

	const cards = purposes
		.map((purpose, index) =>
			buildCard(
				index,
				purpose.reason,
				sources.filter((source) => source.purpose === index && source.shown),
			),
		)
		.filter((card): card is WidgetPurposeView => card !== null)
		.sort(compareCards);

	const included = cards.flatMap((card) =>
		card.sources.filter((source) => source.included),
	);
	const top = included.reduce<WidgetConsentSource | null>(
		(best, source) =>
			!best || rank(source.level) > rank(best.level) ? source : best,
		null,
	);
	const networkLevel = maxLevel(included);
	const capabilities = declaredCovered ? [] : requestedCapabilities;
	const tone =
		maxWidgetSourceLevel([
			...(networkLevel ? [networkLevel] : []),
			...capabilities.map((name) => WIDGET_CAPABILITY_TONE[name]),
		]) ?? "known";
	const hasNetwork = included.length > 0;
	const expanded =
		prompt.mode === "mount" &&
		!declaredCovered &&
		previouslyGranted &&
		(prompt.newSources.length > 0 ||
			prompt.raisedSources.length > 0 ||
			prompt.newCapabilities.length > 0);
	const title: WidgetConsentTitle =
		networkLevel === "broad"
			? "broad"
			: prompt.mode === "runtime" || (declaredCovered && hasNetwork)
				? "runtime"
				: expanded
					? "expanded"
					: hasNetwork
						? "network"
						: "capabilities";
	const shown = sources.filter((source) => source.shown);

	return {
		cards,
		sources,
		capabilities,
		networkLevel,
		tone,
		title,
		lead: bannerLead(top, included, classified),
		hasNetwork,
		top,
		expanded,
		alreadyAllowed: distinct(
			sources.filter((source) => !source.shown).map(chipKey),
		).length,
		runtimeCount: distinct(prompt.runtime.pending.map((entry) => entry.source))
			.length,
		hostCount: distinct(included.map(chipKey)).length,
		classified,
		wildcardUnsupported:
			prompt.descriptor?.engine?.wildcardSources === false &&
			sources.some((source) => source.host.startsWith("*.")),
		loadsPassively: shown.some((source) =>
			source.directives.some((directive) => directive !== "connectSrc"),
		),
		usesWebSockets: shown.some((source) => source.source.startsWith("wss://")),
	};
}

/**
 * What the dialog shows, apart from the viewer's own runtime box toggle. A
 * change is new content under the pointer and restarts the inert window.
 */
export function widgetConsentViewSignature(
	prompt: MicroWidgetConsentPrompt,
	view: WidgetConsentView,
): string {
	const chips = (list: readonly WidgetConsentChip[]) =>
		list.map((chip) => [chip.key, chip.level, chip.isNew, chip.raised]);
	return JSON.stringify([
		prompt.key,
		view.tone,
		view.cards.map((card) => [
			card.index,
			card.level,
			chips(card.declared),
			chips(card.runtime),
		]),
		view.capabilities,
		view.alreadyAllowed,
	]);
}

/**
 * Chips a dialog card shows before "+N more" (§14.5.2). Runtime chips, the
 * first chip of every emphasis domain above `known`, and new or raised ones
 * always show; the rest fill up to `limit`.
 */
export function partitionWidgetChips(
	chips: readonly WidgetConsentChip[],
	limit = WIDGET_CONSENT_VISIBLE_CHIPS,
): { visible: WidgetConsentChip[]; hidden: WidgetConsentChip[] } {
	const keep = new Set<WidgetConsentChip>();
	const domains = new Set<string>();
	for (const chip of chips) {
		const elevated = chip.level !== "known" || chip.isNew || chip.raised;
		if (chip.runtime || (elevated && !domains.has(chip.emphasis))) {
			keep.add(chip);
			domains.add(chip.emphasis);
		}
	}
	for (const chip of chips) {
		if (keep.size >= limit) break;
		keep.add(chip);
	}
	return {
		visible: chips.filter((chip) => keep.has(chip)),
		hidden: chips.filter((chip) => !keep.has(chip)),
	};
}

/** What the blocked card says about a refused prompt (§14.5.5). */
export interface WidgetConsentSummary {
	level: WidgetSourceLevel | null;
	hostCount: number;
	includesRuntime: boolean;
}

export function summarizeWidgetConsent(
	prompt: MicroWidgetConsentPrompt,
	view: WidgetConsentView = buildWidgetConsentView(prompt),
): WidgetConsentSummary {
	return {
		level: view.networkLevel,
		hostCount: view.hostCount,
		includesRuntime: prompt.includeRuntime && prompt.runtime.pending.length > 0,
	};
}
