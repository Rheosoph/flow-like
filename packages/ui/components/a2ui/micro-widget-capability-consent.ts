import type { WidgetContract } from "@flow-like/widget-sdk";
import {
	MICRO_WIDGET_CAPABILITIES,
	type MicroWidgetCapability,
	WIDGET_CSP_KEYS,
	WIDGET_POLICY_SOURCE_ANY_REGISTRY,
	type WidgetCspKey,
	type WidgetPolicy,
	type WidgetRuntimeSourceEntry,
	type WidgetSourceLevel,
	capabilityPolicy,
	isEmptyPolicy,
	isRegistryPolicySource,
	isWidgetSourceLevel,
	normalizeWidgetPolicy,
	policyCapabilities,
	policyCovers,
	policySourceEntries,
	policySources,
	readWidgetPolicy,
	widgetSourceLevelCovers,
	widgetSourceLevelRank,
} from "./micro-widget-policy";

export { MICRO_WIDGET_CAPABILITIES, type MicroWidgetCapability };

export interface MicroWidgetConsentTarget {
	/** The descriptor's `source`, so a grant for one registry never covers another. */
	source: string;
	appId?: string | null;
	packageId: string;
	widgetId: string;
}

export type MicroWidgetConsentScope = "session" | "app";
export type MicroWidgetConsentStatus = "granted" | "pending" | "blocked";

export type ConsentStorage = Pick<
	Storage,
	"getItem" | "setItem" | "removeItem" | "key" | "length"
>;

/** One approved runtime source for one directive. `at` orders LRU eviction and shows the age. */
export interface MicroWidgetRuntimeConsent {
	d: WidgetCspKey;
	s: string;
	l: WidgetSourceLevel;
	at: number;
}

export interface MicroWidgetConsentRecord {
	v: 2;
	/** Declared part; replaced on every allow. */
	policy: WidgetPolicy;
	/** Exactly the sources of `policy.csp`. */
	levels: Record<string, WidgetSourceLevel>;
	runtime?: MicroWidgetRuntimeConsent[];
	runtimeMuted?: true;
	grantedAt: number;
}

/**
 * What a descriptor asks for, split as in §14.4.8: the declared part with the
 * level of each source, and the runtime sources with theirs.
 */
export interface MicroWidgetConsentRequest {
	policy: WidgetPolicy;
	levels: Readonly<Record<string, WidgetSourceLevel>>;
	runtime: readonly WidgetRuntimeSourceEntry[];
}

/**
 * A request as a consent dialog rendered it. Without `shown` the dialog
 * showed all of it.
 */
export interface MicroWidgetConsentGrant extends MicroWidgetConsentRequest {
	shown?: {
		/** False when a live grant already covered the declared part and the dialog hid it. */
		declared: boolean;
		/** Runtime entries the dialog listed as pending. */
		runtime: readonly WidgetRuntimeSourceEntry[];
	};
}

export interface MicroWidgetConsentEvaluation {
	status: MicroWidgetConsentStatus;
	declaredCovered: boolean;
	coveredRuntime: WidgetRuntimeSourceEntry[];
	uncoveredRuntime: WidgetRuntimeSourceEntry[];
	/** Sources granted before at a lower level than now requested. */
	raisedSources: string[];
	/** Sources no live grant mentions. */
	newSources: string[];
	newCapabilities: MicroWidgetCapability[];
	/** "Stop asking": no extraction, no runtime describe, declared-only mounts. */
	muted: boolean;
}

export interface MicroWidgetConsentEntry {
	target: MicroWidgetConsentTarget;
	policy: WidgetPolicy;
	levels: Record<string, WidgetSourceLevel>;
	runtime: MicroWidgetRuntimeConsent[];
	runtimeMuted: boolean;
	/** 0 for grants carried over from the capability-only store. */
	grantedAt: number;
	scope: MicroWidgetConsentScope;
	legacy: boolean;
}

/** Without `appId` the filter matches every app on this device. */
export interface MicroWidgetConsentFilter {
	appId?: string;
}

/** Runtime entries kept per record; the least recently approved or minted go first. */
export const MICRO_WIDGET_RUNTIME_CONSENT_LIMIT = 64;
/** A covered runtime entry's `at` moves forward on mint at most this often. */
export const MICRO_WIDGET_RUNTIME_REFRESH_MS = 86_400_000;

const STORAGE_PREFIX = "widget-consent:v2:";
const GRANT_PREFIX = `${STORAGE_PREFIX}[`;
const REVOKED_PREFIX = `${STORAGE_PREFIX}revoked:`;
const REVOKED_ALL = `${REVOKED_PREFIX}*`;
const LEGACY_PREFIX = "widget-capability-consent-app";

interface SessionGrant {
	target: MicroWidgetConsentTarget;
	record: MicroWidgetConsentRecord;
}

interface RuntimeBlocks {
	target: MicroWidgetConsentTarget;
	sources: Set<string>;
}

const sessionGrants = new Map<string, SessionGrant>();
const sessionBlocks = new Map<string, MicroWidgetConsentTarget>();
const sessionRuntimeBlocks = new Map<string, RuntimeBlocks>();
const listeners = new Set<() => void>();
const NO_SOURCES: ReadonlySet<string> = new Set();

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCapability(value: unknown): value is MicroWidgetCapability {
	return (
		typeof value === "string" &&
		(MICRO_WIDGET_CAPABILITIES as readonly string[]).includes(value)
	);
}

function isCspKey(value: unknown): value is WidgetCspKey {
	return (
		typeof value === "string" &&
		(WIDGET_CSP_KEYS as readonly string[]).includes(value)
	);
}

/**
 * Capabilities a page-JSON contract asks for. Only used when the backend
 * cannot describe the widget and the host falls back to the legacy frame.
 * Preview surfaces never capture or play audio.
 */
export function readRequestedCapabilities(
	contract: WidgetContract | null | undefined,
	preview: boolean,
): MicroWidgetCapability[] {
	const capabilities: unknown = contract?.capabilities;
	if (
		!capabilities ||
		typeof capabilities !== "object" ||
		Array.isArray(capabilities)
	) {
		return [];
	}
	return MICRO_WIDGET_CAPABILITIES.filter((name) => {
		if (preview && (name === "microphone" || name === "media")) return false;
		return (
			Object.prototype.hasOwnProperty.call(capabilities, name) &&
			(capabilities as Record<string, unknown>)[name] === true
		);
	});
}

/**
 * A request for a bare policy, as older callers pass it: every source at the
 * worst level and no runtime part, so it is covered only by a grant that
 * accepted every level.
 */
export function microWidgetConsentRequest(
	value: MicroWidgetConsentGrant | WidgetPolicy,
): MicroWidgetConsentGrant {
	if (
		isRecord(value) &&
		"policy" in value &&
		"levels" in value &&
		"runtime" in value
	) {
		return value as unknown as MicroWidgetConsentGrant;
	}
	const policy = value as WidgetPolicy;
	const levels: Record<string, WidgetSourceLevel> = {};
	for (const [, source] of policySourceEntries(policy))
		levels[source] = "broad";
	return { policy, levels, runtime: [] };
}

/** One unambiguous key per (source, app, package, widget), shared by storage and the session map. */
export function microWidgetConsentKey(
	target: MicroWidgetConsentTarget,
): string {
	return `${STORAGE_PREFIX}${JSON.stringify([
		target.source,
		target.appId ?? null,
		target.packageId,
		target.widgetId,
	])}`;
}

export function microWidgetConsentRevokedKey(
	target: MicroWidgetConsentTarget,
): string {
	return `${REVOKED_PREFIX}${microWidgetConsentKey(target)}`;
}

function appRevokedKey(appId: string | null | undefined): string {
	return `${REVOKED_PREFIX}app:${JSON.stringify(appId ?? null)}`;
}

function packageRevokedKey(packageId: string): string {
	return `${REVOKED_PREFIX}pkg:${JSON.stringify(packageId)}`;
}

function defaultStorage(): ConsentStorage | null {
	try {
		return typeof localStorage === "undefined" ? null : localStorage;
	} catch {
		return null;
	}
}

function safeGet(storage: ConsentStorage | null, key: string): string | null {
	if (!storage) return null;
	try {
		return storage.getItem(key);
	} catch {
		return null;
	}
}

function safeSet(storage: ConsentStorage | null, key: string, value: string) {
	if (!storage) return;
	try {
		storage.setItem(key, value);
	} catch {
		// Storage unavailable: session state still applies.
	}
}

function safeRemove(storage: ConsentStorage | null, key: string) {
	if (!storage) return;
	try {
		storage.removeItem(key);
	} catch {
		// Storage unavailable: nothing persisted to remove.
	}
}

function storageKeys(storage: ConsentStorage | null): string[] {
	if (!storage) return [];
	try {
		const keys: string[] = [];
		for (let index = 0; index < storage.length; index++) {
			const key = storage.key(index);
			if (key !== null) keys.push(key);
		}
		return keys;
	} catch {
		return [];
	}
}

function readEpoch(storage: ConsentStorage | null, key: string): number {
	const value = Number(safeGet(storage, key));
	return Number.isFinite(value) && value > 0 ? value : 0;
}

/** Latest revocation that applies to the target: its own, its app's, its package's, or device-wide. */
function revokedAt(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null,
): number {
	return Math.max(
		readEpoch(storage, microWidgetConsentRevokedKey(target)),
		readEpoch(storage, appRevokedKey(target.appId)),
		readEpoch(storage, packageRevokedKey(target.packageId)),
		readEpoch(storage, REVOKED_ALL),
	);
}

/** Later than every revocation on this device, so it also outlives grants given right after one. */
function nextRevocationEpoch(
	storage: ConsentStorage | null,
	keys: readonly string[],
): string {
	const latest = keys
		.filter((key) => key.startsWith(REVOKED_PREFIX))
		.reduce((max, key) => Math.max(max, readEpoch(storage, key)), 0);
	return String(Math.max(Date.now(), latest + 1));
}

function readLevels(
	value: unknown,
	policy: WidgetPolicy,
): Record<string, WidgetSourceLevel> {
	const levels: Record<string, WidgetSourceLevel> = {};
	if (!isRecord(value)) return levels;
	for (const [, source] of policySourceEntries(policy)) {
		const level = Object.prototype.hasOwnProperty.call(value, source)
			? value[source]
			: undefined;
		if (isWidgetSourceLevel(level)) levels[source] = level;
	}
	return levels;
}

function isRuntimeConsent(value: unknown): value is MicroWidgetRuntimeConsent {
	if (!isRecord(value)) return false;
	const { d, s, l, at } = value;
	return (
		isCspKey(d) &&
		typeof s === "string" &&
		s.length > 0 &&
		isWidgetSourceLevel(l) &&
		typeof at === "number" &&
		Number.isFinite(at) &&
		at > 0
	);
}

function runtimeKey(directive: WidgetCspKey, source: string): string {
	return `${directive} ${source}`;
}

/** Latest entry per (directive, source), newest first, at most the LRU limit. */
function compactRuntime(
	entries: readonly MicroWidgetRuntimeConsent[],
): MicroWidgetRuntimeConsent[] {
	const latest = new Map<string, MicroWidgetRuntimeConsent>();
	for (const entry of entries) {
		const key = runtimeKey(entry.d, entry.s);
		const previous = latest.get(key);
		if (!previous || entry.at >= previous.at) latest.set(key, entry);
	}
	return [...latest.values()]
		.sort((left, right) => right.at - left.at)
		.slice(0, MICRO_WIDGET_RUNTIME_CONSENT_LIMIT);
}

function readRuntime(value: unknown): MicroWidgetRuntimeConsent[] {
	return Array.isArray(value)
		? compactRuntime(
				value
					.filter(isRuntimeConsent)
					.map(({ d, s, l, at }) => ({ d, s, l, at })),
			)
		: [];
}

/** Lenient: a malformed `levels` leaves its sources uncovered and a malformed `runtime` is ignored alone. */
function parseRecord(value: unknown): MicroWidgetConsentRecord | null {
	if (!isRecord(value)) return null;
	const policy = readWidgetPolicy(value.policy);
	const { grantedAt } = value;
	if (
		value.v !== 2 ||
		!policy ||
		typeof grantedAt !== "number" ||
		!Number.isFinite(grantedAt)
	) {
		return null;
	}
	return buildRecord(
		policy,
		readLevels(value.levels, policy),
		readRuntime(value.runtime),
		value.runtimeMuted === true,
		grantedAt,
	);
}

function buildRecord(
	policy: WidgetPolicy,
	levels: Record<string, WidgetSourceLevel>,
	runtime: readonly MicroWidgetRuntimeConsent[],
	muted: boolean,
	grantedAt: number,
): MicroWidgetConsentRecord {
	const record: MicroWidgetConsentRecord = { v: 2, policy, levels, grantedAt };
	const compacted = compactRuntime(runtime);
	if (compacted.length > 0) record.runtime = compacted;
	if (muted) record.runtimeMuted = true;
	return record;
}

function readRecord(
	storage: ConsentStorage | null,
	key: string,
): MicroWidgetConsentRecord | null {
	const raw = safeGet(storage, key);
	if (!raw) return null;
	try {
		return parseRecord(JSON.parse(raw));
	} catch {
		return null;
	}
}

function writeRecord(
	storage: ConsentStorage | null,
	key: string,
	record: MicroWidgetConsentRecord,
): void {
	safeSet(storage, key, JSON.stringify(record));
}

function parseGrantKey(key: string): MicroWidgetConsentTarget | null {
	if (!key.startsWith(GRANT_PREFIX)) return null;
	try {
		const parsed: unknown = JSON.parse(key.slice(STORAGE_PREFIX.length));
		if (!Array.isArray(parsed) || parsed.length !== 4) return null;
		const [source, appId, packageId, widgetId] = parsed;
		if (
			typeof source !== "string" ||
			(appId !== null && typeof appId !== "string") ||
			typeof packageId !== "string" ||
			typeof widgetId !== "string"
		) {
			return null;
		}
		return appId === null
			? { source, packageId, widgetId }
			: { source, appId, packageId, widgetId };
	} catch {
		return null;
	}
}

/**
 * The capability-only store keyed grants as `<prefix>-<appId>-<packageId>/<widgetId>`.
 * That split is only unambiguous when neither id contains `-` (app ids are
 * cuid2), so any other target re-prompts instead of inheriting a grant.
 */
function legacyKey(target: MicroWidgetConsentTarget): string | null {
	if (!isRegistryPolicySource(target.source)) return null;
	const { appId, packageId, widgetId } = target;
	if (!appId || appId.includes("-") || packageId.includes("-")) return null;
	return `${LEGACY_PREFIX}-${appId}-${packageId}/${widgetId}`;
}

function parseLegacyKey(key: string): MicroWidgetConsentTarget | null {
	if (!key.startsWith(`${LEGACY_PREFIX}-`)) return null;
	const rest = key.slice(LEGACY_PREFIX.length + 1);
	const slash = rest.indexOf("/");
	if (slash < 0) return null;
	const owner = rest.slice(0, slash).split("-");
	const widgetId = rest.slice(slash + 1);
	if (owner.length !== 2 || !owner[0] || !owner[1] || !widgetId) return null;
	return {
		source: WIDGET_POLICY_SOURCE_ANY_REGISTRY,
		appId: owner[0],
		packageId: owner[1],
		widgetId,
	};
}

function readLegacyGrant(
	storage: ConsentStorage | null,
	key: string,
): WidgetPolicy | null {
	const raw = safeGet(storage, key);
	if (!raw) return null;
	try {
		const parsed: unknown = JSON.parse(raw);
		return Array.isArray(parsed)
			? capabilityPolicy(parsed.filter(isCapability))
			: null;
	} catch {
		return null;
	}
}

function notify(): void {
	for (const listener of listeners) listener();
}

function isConsentStorageKey(key: unknown): boolean {
	return (
		typeof key !== "string" ||
		key.startsWith(STORAGE_PREFIX) ||
		key.startsWith(LEGACY_PREFIX)
	);
}

function onStorage(event: Event): void {
	if (isConsentStorageKey((event as StorageEvent).key)) notify();
}

function onPageShow(event: Event): void {
	if ((event as PageTransitionEvent).persisted) notify();
}

let attached: EventTarget | null = null;

/**
 * Other tabs write grants, blocks and revocations to shared storage; a page
 * restored from the back-forward cache may have missed those events.
 */
export function subscribeMicroWidgetConsent(listener: () => void): () => void {
	listeners.add(listener);
	if (!attached && typeof window !== "undefined") {
		attached = window;
		attached.addEventListener("storage", onStorage);
		attached.addEventListener("pageshow", onPageShow);
	}
	return () => {
		listeners.delete(listener);
		if (listeners.size === 0 && attached) {
			attached.removeEventListener("storage", onStorage);
			attached.removeEventListener("pageshow", onPageShow);
			attached = null;
		}
	};
}

interface LiveGrants {
	/** Records given after the latest revocation. */
	records: MicroWidgetConsentRecord[];
	/** Runtime entries approved after the latest revocation, from any record. */
	runtime: MicroWidgetRuntimeConsent[];
	legacy: WidgetPolicy | null;
}

function liveGrants(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null,
): LiveGrants {
	const key = microWidgetConsentKey(target);
	const revoked = revokedAt(target, storage);
	const candidates = [
		sessionGrants.get(key)?.record ?? null,
		readRecord(storage, key),
	].filter((record): record is MicroWidgetConsentRecord => record !== null);
	const legacy = legacyKey(target);
	return {
		records: candidates.filter((record) => record.grantedAt > revoked),
		runtime: candidates.flatMap((record) =>
			(record.runtime ?? []).filter((entry) => entry.at > revoked),
		),
		legacy: legacy && revoked === 0 ? readLegacyGrant(storage, legacy) : null,
	};
}

function declaredCoveredBy(
	record: MicroWidgetConsentRecord,
	request: MicroWidgetConsentRequest,
): boolean {
	const granted = policyCapabilities(record.policy);
	if (
		!policyCapabilities(request.policy).every((name) => granted.includes(name))
	)
		return false;
	return policySourceEntries(request.policy).every(
		([directive, source]) =>
			policySources(record.policy, directive).includes(source) &&
			widgetSourceLevelCovers(
				record.levels[source],
				request.levels[source] ?? "broad",
			),
	);
}

function runtimeCovered(
	entry: WidgetRuntimeSourceEntry,
	live: LiveGrants,
): boolean {
	return (
		live.runtime.some(
			(granted) =>
				granted.d === entry.directive &&
				granted.s === entry.source &&
				widgetSourceLevelCovers(granted.l, entry.level),
		) ||
		live.records.some(
			(record) =>
				policySources(record.policy, entry.directive).includes(entry.source) &&
				widgetSourceLevelCovers(record.levels[entry.source], entry.level),
		)
	);
}

interface GrantedMention {
	mentioned: boolean;
	/** Riskiest level any live grant accepted for the source; null when none recorded one. */
	level: WidgetSourceLevel | null;
}

function grantedMention(source: string, live: LiveGrants): GrantedMention {
	const levels: (WidgetSourceLevel | undefined)[] = [
		...live.runtime
			.filter((entry) => entry.s === source)
			.map((entry) => entry.l),
		...live.records
			.filter((record) =>
				policySourceEntries(record.policy).some(
					([, granted]) => granted === source,
				),
			)
			.map((record) => record.levels[source]),
	];
	const known = levels.filter(isWidgetSourceLevel);
	return {
		mentioned: levels.length > 0,
		level:
			known.length === 0
				? null
				: known.reduce((max, level) =>
						widgetSourceLevelRank(level) > widgetSourceLevelRank(max)
							? level
							: max,
					),
	};
}

/**
 * Coverage per §14.4.8 (approvals never expire). The declared part needs one
 * live record that holds every capability and every (directive, source) at a
 * level at least as risky as requested; a missing level is not covered.
 * Each runtime source needs a runtime entry approved after the latest
 * revocation, or a live record's declared policy, at a sufficient level.
 * Runtime entries never cover the declared part.
 */
export function evaluateMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
	value: MicroWidgetConsentRequest | WidgetPolicy,
	storage: ConsentStorage | null = defaultStorage(),
): MicroWidgetConsentEvaluation {
	const request = microWidgetConsentRequest(value);
	const live = liveGrants(target, storage);
	const declaredCovered =
		isEmptyPolicy(request.policy) ||
		live.records.some((record) => declaredCoveredBy(record, request)) ||
		policyCovers(live.legacy, request.policy);
	const coveredRuntime: WidgetRuntimeSourceEntry[] = [];
	const uncoveredRuntime: WidgetRuntimeSourceEntry[] = [];
	for (const entry of request.runtime) {
		(runtimeCovered(entry, live) ? coveredRuntime : uncoveredRuntime).push(
			entry,
		);
	}
	const requestedLevels = new Map<string, WidgetSourceLevel>();
	for (const [, source] of policySourceEntries(request.policy)) {
		requestedLevels.set(source, request.levels[source] ?? "broad");
	}
	for (const entry of request.runtime) {
		const previous = requestedLevels.get(entry.source);
		if (
			!previous ||
			widgetSourceLevelRank(entry.level) > widgetSourceLevelRank(previous)
		) {
			requestedLevels.set(entry.source, entry.level);
		}
	}
	const raisedSources: string[] = [];
	const newSources: string[] = [];
	for (const [source, level] of requestedLevels) {
		const granted = grantedMention(source, live);
		if (!granted.mentioned) newSources.push(source);
		else if (granted.level && !widgetSourceLevelCovers(granted.level, level))
			raisedSources.push(source);
	}
	const grantedCapabilities = new Set([
		...live.records.flatMap((record) => policyCapabilities(record.policy)),
		...policyCapabilities(live.legacy),
	]);
	const granted = declaredCovered && uncoveredRuntime.length === 0;
	const key = microWidgetConsentKey(target);
	return {
		status: granted
			? "granted"
			: sessionBlocks.has(key)
				? "blocked"
				: "pending",
		declaredCovered,
		coveredRuntime,
		uncoveredRuntime,
		raisedSources: raisedSources.sort(),
		newSources: newSources.sort(),
		newCapabilities: policyCapabilities(request.policy).filter(
			(name) => !grantedCapabilities.has(name),
		),
		muted: live.records.some((record) => record.runtimeMuted === true),
	};
}

/** Status of `evaluateMicroWidgetConsent`. An empty request needs no grant. */
export function readMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
	value: MicroWidgetConsentRequest | WidgetPolicy,
	storage: ConsentStorage | null = defaultStorage(),
): MicroWidgetConsentStatus {
	return evaluateMicroWidgetConsent(target, value, storage).status;
}

function liveRuntimeOf(
	record: MicroWidgetConsentRecord | null | undefined,
	revoked: number,
): MicroWidgetRuntimeConsent[] {
	return (record?.runtime ?? []).filter((entry) => entry.at > revoked);
}

function liveMuted(
	record: MicroWidgetConsentRecord | null | undefined,
	revoked: number,
): boolean {
	return (
		record !== null &&
		record !== undefined &&
		record.grantedAt > revoked &&
		record.runtimeMuted === true
	);
}

/**
 * Grants the rendered request. A shown declared part and its levels replace
 * what was granted, so hosts a widget no longer asks for do not linger; a
 * hidden one leaves every record's declared part as it was. Runtime entries
 * are merged into those still valid, stamped now. A project grant is also
 * kept for this session, but the project record only takes what the dialog
 * showed: nothing allowed for this session alone becomes permanent unseen.
 * Without an app only the session grant exists.
 */
export function grantMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
	value: MicroWidgetConsentGrant | WidgetPolicy,
	scope: MicroWidgetConsentScope,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	const request = microWidgetConsentRequest(value);
	const declaredShown = request.shown?.declared ?? true;
	const key = microWidgetConsentKey(target);
	const revoked = revokedAt(target, storage);
	const grantedAt = Math.max(Date.now(), revoked + 1);
	const policy = normalizeWidgetPolicy(request.policy);
	const levels: Record<string, WidgetSourceLevel> = {};
	for (const [, source] of policySourceEntries(policy)) {
		levels[source] = request.levels[source] ?? "broad";
	}
	const stamp = (entries: readonly WidgetRuntimeSourceEntry[]) =>
		entries.map(
			(entry): MicroWidgetRuntimeConsent => ({
				d: entry.directive,
				s: entry.source,
				l: entry.level,
				at: grantedAt,
			}),
		);
	const merge = (
		previous: MicroWidgetConsentRecord | null | undefined,
		entries: readonly WidgetRuntimeSourceEntry[],
	): MicroWidgetConsentRecord => {
		const runtime = [...liveRuntimeOf(previous, revoked), ...stamp(entries)];
		const muted = liveMuted(previous, revoked);
		if (declaredShown) {
			return buildRecord(policy, levels, runtime, muted, grantedAt);
		}
		const live = previous && previous.grantedAt > revoked ? previous : null;
		return buildRecord(
			live?.policy ?? {},
			live?.levels ?? {},
			runtime,
			muted,
			live?.grantedAt ?? grantedAt,
		);
	};
	sessionBlocks.delete(key);
	const blocks = sessionRuntimeBlocks.get(key);
	for (const entry of request.runtime) blocks?.sources.delete(entry.source);
	sessionGrants.set(key, {
		target,
		record: merge(sessionGrants.get(key)?.record, request.runtime),
	});
	if (scope === "app" && target.appId) {
		writeRecord(
			storage,
			key,
			merge(
				readRecord(storage, key),
				request.shown?.runtime ?? request.runtime,
			),
		);
		const legacy = legacyKey(target);
		if (legacy && declaredShown) safeRemove(storage, legacy);
	}
	notify();
}

/**
 * Moves `at` of covered runtime entries to now when the widget is minted with
 * them, at most once a day per entry, so LRU eviction keeps what is in use.
 */
export function refreshMicroWidgetRuntimeConsent(
	target: MicroWidgetConsentTarget,
	entries: readonly Pick<WidgetRuntimeSourceEntry, "directive" | "source">[],
	storage: ConsentStorage | null = defaultStorage(),
	now: number = Date.now(),
): void {
	if (entries.length === 0) return;
	const key = microWidgetConsentKey(target);
	const revoked = revokedAt(target, storage);
	const wanted = new Set(
		entries.map((entry) => runtimeKey(entry.directive, entry.source)),
	);
	const refresh = (record: MicroWidgetConsentRecord): boolean => {
		let changed = false;
		for (const entry of record.runtime ?? []) {
			if (
				wanted.has(runtimeKey(entry.d, entry.s)) &&
				entry.at > revoked &&
				now - entry.at > MICRO_WIDGET_RUNTIME_REFRESH_MS
			) {
				entry.at = now;
				changed = true;
			}
		}
		if (changed && record.runtime)
			record.runtime = compactRuntime(record.runtime);
		return changed;
	};
	const session = sessionGrants.get(key)?.record;
	if (session) refresh(session);
	const stored = readRecord(storage, key);
	if (stored && refresh(stored)) writeRecord(storage, key, stored);
}

function forget(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null,
): void {
	const key = microWidgetConsentKey(target);
	sessionGrants.delete(key);
	sessionRuntimeBlocks.delete(key);
	safeRemove(storage, key);
	const legacy = legacyKey(target);
	if (legacy) safeRemove(storage, legacy);
	safeSet(
		storage,
		microWidgetConsentRevokedKey(target),
		String(Math.max(Date.now(), revokedAt(target, storage) + 1)),
	);
}

/** Clears every grant for the target, in every tab, and keeps it blocked for this session. */
export function blockMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	forget(target, storage);
	sessionBlocks.set(microWidgetConsentKey(target), target);
	notify();
}

/**
 * "Don't allow" on runtime sources while the declared part stays allowed:
 * extraction skips them for the rest of this session. Nothing is forgotten.
 */
export function blockMicroWidgetRuntimeSources(
	target: MicroWidgetConsentTarget,
	sources: Iterable<string>,
): void {
	const key = microWidgetConsentKey(target);
	const blocks = sessionRuntimeBlocks.get(key) ?? {
		target,
		sources: new Set<string>(),
	};
	const before = blocks.sources.size;
	for (const source of sources) blocks.sources.add(source);
	if (blocks.sources.size === before) return;
	sessionRuntimeBlocks.set(key, blocks);
	notify();
}

/** Runtime sources this session refused for the target; they feed extraction's `excluded`. */
export function readMicroWidgetRuntimeBlocks(
	target: MicroWidgetConsentTarget,
): ReadonlySet<string> {
	return (
		sessionRuntimeBlocks.get(microWidgetConsentKey(target))?.sources ??
		NO_SOURCES
	);
}

function setMuted(
	record: MicroWidgetConsentRecord | null | undefined,
	revoked: number,
	muted: boolean,
	grantedAt: number,
): MicroWidgetConsentRecord | null {
	if (record && record.grantedAt > revoked) {
		return buildRecord(
			record.policy,
			record.levels,
			record.runtime ?? [],
			muted,
			record.grantedAt,
		);
	}
	return muted ? buildRecord({}, {}, [], true, grantedAt) : null;
}

function isBareRecord(record: MicroWidgetConsentRecord): boolean {
	return (
		isEmptyPolicy(record.policy) &&
		(record.runtime ?? []).length === 0 &&
		record.runtimeMuted !== true
	);
}

/**
 * "Stop asking": the widget runs without new runtime sources and never asks
 * for them. Stored for the project when there is one, else for this session.
 */
export function muteMicroWidgetRuntime(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	const key = microWidgetConsentKey(target);
	const revoked = revokedAt(target, storage);
	const grantedAt = Math.max(Date.now(), revoked + 1);
	const session = setMuted(
		sessionGrants.get(key)?.record,
		revoked,
		true,
		grantedAt,
	);
	if (session) sessionGrants.set(key, { target, record: session });
	if (target.appId) {
		const stored = setMuted(readRecord(storage, key), revoked, true, grantedAt);
		if (stored) writeRecord(storage, key, stored);
	}
	notify();
}

/** "Ask again": runtime sources prompt again. */
export function unmuteMicroWidgetRuntime(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	const key = microWidgetConsentKey(target);
	const revoked = revokedAt(target, storage);
	const session = setMuted(sessionGrants.get(key)?.record, revoked, false, 0);
	if (session && !isBareRecord(session)) {
		sessionGrants.set(key, { target, record: session });
	} else {
		sessionGrants.delete(key);
	}
	const stored = setMuted(readRecord(storage, key), revoked, false, 0);
	if (stored && !isBareRecord(stored)) writeRecord(storage, key, stored);
	else if (stored) safeRemove(storage, key);
	notify();
}

/** Forgets the decision in every tab; the next mount prompts again. */
export function revokeMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	forget(target, storage);
	sessionBlocks.delete(microWidgetConsentKey(target));
	notify();
}

export function reopenMicroWidgetConsent(
	target: MicroWidgetConsentTarget,
): void {
	const key = microWidgetConsentKey(target);
	sessionBlocks.delete(key);
	sessionRuntimeBlocks.delete(key);
	notify();
}

function matchesFilter(
	target: MicroWidgetConsentTarget,
	filter: MicroWidgetConsentFilter,
): boolean {
	return filter.appId === undefined || target.appId === filter.appId;
}

function compareEntries(
	left: MicroWidgetConsentEntry,
	right: MicroWidgetConsentEntry,
): number {
	const order = (entry: MicroWidgetConsentEntry) =>
		[
			entry.target.appId ?? "",
			entry.target.packageId,
			entry.target.widgetId,
			entry.target.source,
			entry.scope,
		].join(" ");
	return order(left) < order(right) ? -1 : order(left) > order(right) ? 1 : 0;
}

function entryFor(
	target: MicroWidgetConsentTarget,
	record: MicroWidgetConsentRecord,
	revoked: number,
	scope: MicroWidgetConsentScope,
): MicroWidgetConsentEntry {
	return {
		target,
		policy: record.policy,
		levels: record.levels,
		runtime: liveRuntimeOf(record, revoked),
		runtimeMuted: record.runtimeMuted === true,
		grantedAt: record.grantedAt,
		scope,
		legacy: false,
	};
}

function sessionAddsNothing(
	stored: MicroWidgetConsentEntry,
	session: MicroWidgetConsentEntry,
): boolean {
	return (
		policyCovers(stored.policy, session.policy) &&
		policySourceEntries(session.policy).every(([, source]) =>
			widgetSourceLevelCovers(
				stored.levels[source],
				session.levels[source] ?? "broad",
			),
		) &&
		session.runtime.every((entry) =>
			stored.runtime.some(
				(kept) =>
					kept.d === entry.d &&
					kept.s === entry.s &&
					widgetSourceLevelCovers(kept.l, entry.l),
			),
		) &&
		(!session.runtimeMuted || stored.runtimeMuted)
	);
}

/** Grants that currently apply, stored and session, for the manage UI. */
export function listMicroWidgetConsents(
	appId?: string,
	storage: ConsentStorage | null = defaultStorage(),
): MicroWidgetConsentEntry[] {
	const filter: MicroWidgetConsentFilter = { appId };
	const entries = new Map<string, MicroWidgetConsentEntry>();
	for (const key of storageKeys(storage)) {
		const stored = parseGrantKey(key);
		if (stored && matchesFilter(stored, filter)) {
			const record = readRecord(storage, key);
			const revoked = revokedAt(stored, storage);
			if (record && record.grantedAt > revoked) {
				entries.set(key, entryFor(stored, record, revoked, "app"));
			}
			continue;
		}
		const legacy = parseLegacyKey(key);
		if (legacy && matchesFilter(legacy, filter)) {
			const policy = readLegacyGrant(storage, key);
			if (
				policy &&
				!isEmptyPolicy(policy) &&
				revokedAt(legacy, storage) === 0
			) {
				entries.set(key, {
					target: legacy,
					policy,
					levels: {},
					runtime: [],
					runtimeMuted: false,
					grantedAt: 0,
					scope: "app",
					legacy: true,
				});
			}
		}
	}
	for (const [key, grant] of sessionGrants) {
		if (!matchesFilter(grant.target, filter)) continue;
		const revoked = revokedAt(grant.target, storage);
		if (grant.record.grantedAt <= revoked) continue;
		const session = entryFor(grant.target, grant.record, revoked, "session");
		const stored = entries.get(key);
		if (stored && sessionAddsNothing(stored, session)) continue;
		entries.set(`${key}:session`, session);
	}
	return [...entries.values()].sort(compareEntries);
}

/**
 * Revokes every grant for one app, or on the whole device, including session
 * grants held by other tabs that this tab has never seen.
 */
export function clearMicroWidgetConsents(
	filter: MicroWidgetConsentFilter = {},
	storage: ConsentStorage | null = defaultStorage(),
): void {
	const keys = storageKeys(storage);
	safeSet(
		storage,
		filter.appId === undefined ? REVOKED_ALL : appRevokedKey(filter.appId),
		nextRevocationEpoch(storage, keys),
	);
	const legacyOwner =
		filter.appId === undefined
			? `${LEGACY_PREFIX}-`
			: `${LEGACY_PREFIX}-${filter.appId}-`;
	for (const key of keys) {
		const stored = parseGrantKey(key);
		if (
			(stored && matchesFilter(stored, filter)) ||
			key.startsWith(legacyOwner)
		) {
			safeRemove(storage, key);
		}
	}
	for (const [key, grant] of sessionGrants) {
		if (matchesFilter(grant.target, filter)) sessionGrants.delete(key);
	}
	for (const [key, target] of sessionBlocks) {
		if (matchesFilter(target, filter)) sessionBlocks.delete(key);
	}
	for (const [key, blocks] of sessionRuntimeBlocks) {
		if (matchesFilter(blocks.target, filter)) sessionRuntimeBlocks.delete(key);
	}
	notify();
}

/**
 * Revokes every grant for one package's widgets, in every app and source,
 * including session grants held by other tabs that this tab has never seen.
 */
export function clearPackageMicroWidgetConsents(
	packageId: string,
	storage: ConsentStorage | null = defaultStorage(),
): void {
	const keys = storageKeys(storage);
	safeSet(
		storage,
		packageRevokedKey(packageId),
		nextRevocationEpoch(storage, keys),
	);
	for (const key of keys) {
		const owner = parseGrantKey(key) ?? parseLegacyKey(key);
		if (owner?.packageId === packageId) safeRemove(storage, key);
	}
	for (const [key, grant] of sessionGrants) {
		if (grant.target.packageId === packageId) sessionGrants.delete(key);
	}
	for (const [key, target] of sessionBlocks) {
		if (target.packageId === packageId) sessionBlocks.delete(key);
	}
	for (const [key, blocks] of sessionRuntimeBlocks) {
		if (blocks.target.packageId === packageId) sessionRuntimeBlocks.delete(key);
	}
	notify();
}

export function resetMicroWidgetConsentForTests(): void {
	sessionGrants.clear();
	sessionBlocks.clear();
	sessionRuntimeBlocks.clear();
}
