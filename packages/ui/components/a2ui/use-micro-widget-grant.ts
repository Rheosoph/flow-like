"use client";

import type { WidgetContract } from "@flow-like/widget-sdk";
import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { useBackend, useBackendReady } from "../../state/backend-state";
import type { IRegistryState } from "../../state/backend-state/registry-state";
import {
	type MicroWidgetCapability,
	type MicroWidgetConsentEvaluation,
	type MicroWidgetConsentRequest,
	type MicroWidgetConsentScope,
	type MicroWidgetConsentStatus,
	type MicroWidgetConsentTarget,
	blockMicroWidgetConsent,
	blockMicroWidgetRuntimeSources,
	evaluateMicroWidgetConsent,
	grantMicroWidgetConsent,
	muteMicroWidgetRuntime,
	readMicroWidgetRuntimeBlocks,
	readRequestedCapabilities,
	refreshMicroWidgetRuntimeConsent,
	reopenMicroWidgetConsent,
	subscribeMicroWidgetConsent,
} from "./micro-widget-capability-consent";
import {
	buildDesktopMicroWidgetFrameSrc,
	buildDesktopMicroWidgetSrc,
	buildWebMicroWidgetFramePath,
	buildWebMicroWidgetPath,
} from "./micro-widget-host";
import { publicWidgetProps } from "./micro-widget-media";
import {
	WIDGET_POLICY_SOURCE_ANY_REGISTRY,
	type WidgetDeclaredPart,
	type WidgetDescriptorSplit,
	type WidgetPolicy,
	type WidgetPolicyDescriptor,
	type WidgetRuntimeRejection,
	type WidgetRuntimeSourceEntry,
	type WidgetRuntimeSourceRequest,
	type WidgetSourceLevel,
	canonicalWidgetRuntimeRequest,
	capabilityPolicy,
	isEmptyPolicy,
	isPolicyChangedError,
	isWidgetGrantUnavailableError,
	isWidgetRuntimeDescribeUnsupportedError,
	maxWidgetSourceLevel,
	policyCapabilities,
	splitWidgetPolicyDescriptor,
	widgetRuntimeRequestKey,
	widgetRuntimeRequestSources,
	widgetRuntimeSourcesErrorCode,
} from "./micro-widget-policy";
import {
	RUNTIME_EXTRACTION_LIMITS,
	type RuntimeValueIssue,
	extractRuntimeSources,
} from "./micro-widget-runtime-sources";

/** A cached grant is reused only while it has at least this long left. */
export const MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS = 5 * 60_000;
/** A mount re-mints an expired grant on frame load at most this often. */
export const MICRO_WIDGET_REMINT_INTERVAL_MS = 60_000;
/** Minted grants kept on this page, least recently used first out. */
export const MICRO_WIDGET_GRANT_CACHE_LIMIT = 256;
/** An empty first extraction waits this long for the first props patch. */
export const MICRO_WIDGET_RUNTIME_GRACE_MS = 500;
/** Props changes settle this long before runtime sources are extracted again. */
export const MICRO_WIDGET_RUNTIME_DEBOUNCE_MS = 250;
/** A stream of props changes is extracted at least this often. */
export const MICRO_WIDGET_RUNTIME_MAX_WAIT_MS = 1_000;
/** Once the widget said hello, its document is replaced at most this often. */
export const MICRO_WIDGET_SWAP_INTERVAL_MS = 5_000;
/** First retry delay after runtime sources were unavailable; doubles per miss. */
export const MICRO_WIDGET_RUNTIME_BACKOFF_MS = 30_000;
export const MICRO_WIDGET_RUNTIME_MAX_BACKOFF_MS = 8 * 60_000;

const BASELINE_POLICY: WidgetPolicy = {};
const NO_RUNTIME: WidgetRuntimeSourceRequest[] = [];

/** What the consent dialog and the blocked card render. */
export type MicroWidgetPolicySubject = Pick<
	WidgetPolicyDescriptor,
	"source" | "packageId" | "widgetId" | "policy"
>;

export interface MicroWidgetGrantInput {
	packageId: string;
	packageVersion: string;
	bundleHash: string | null | undefined;
	widgetId: string;
	preview: boolean;
	appId: string | null | undefined;
	/** Page JSON contract. Read only to decide the fallback when the backend cannot describe. */
	contract: WidgetContract | null | undefined;
	/** The instance's props; runtime sources are extracted from what the widget receives. */
	props: Record<string, unknown> | null | undefined;
	/** False while the widget cannot be addressed (desktop without a bundle hash). */
	enabled: boolean;
}

/**
 * `invalid`: the backend could not verify the declared policy.
 * `baseline`: the user blocked the policy and chose to run without it.
 * `unavailable`: the backend cannot issue grants.
 */
export type MicroWidgetFrameNotice = "invalid" | "baseline" | "unavailable";

export interface MicroWidgetFrameMount {
	/** `grant` serves `frame/{wid}/{grant|0}`; `legacy` the pre-grant URL for servers that cannot describe. */
	kind: "grant" | "legacy";
	packageId: string;
	packageVersion: string;
	bundleHash: string | null;
	widgetId: string;
	grant: string | null;
	/** Web runtime component minted with `grant` (`{grant}~{runtime}`); always null on desktop. */
	runtime: string | null;
	/** The policy the document runs under. Drives the sandbox attribute and init capabilities. */
	policy: WidgetPolicy;
	notice: MicroWidgetFrameNotice | null;
	invalidReason: string | null;
}

export type MicroWidgetGrantError =
	| { reason: "policy_unstable" }
	| { reason: "mint_failed"; detail: string };

export type MicroWidgetGrantState =
	| { status: "describing" }
	| { status: "unsupported"; detail: string | null }
	| { status: "pending"; subject: MicroWidgetPolicySubject }
	| {
			status: "blocked";
			subject: MicroWidgetPolicySubject;
			canRunBaseline: boolean;
	  }
	| { status: "minting"; subject: MicroWidgetPolicySubject }
	| { status: "ready"; frame: MicroWidgetFrameMount }
	| ({ status: "error" } & MicroWidgetGrantError);

/**
 * The frozen request a consent dialog renders. `mount`: nothing runs yet.
 * `runtime`: the widget runs and new runtime sources wait for review. Every
 * allow grants and mints exactly this `descriptor` with its runtime sources.
 */
export interface MicroWidgetConsentPrompt {
	/** Identifies what this prompt shows; decisions carry it so they never apply to content the viewer did not see. */
	key: string;
	mode: "mount" | "runtime";
	/** Null only for the capability prompt of servers that cannot describe. */
	descriptor: WidgetPolicyDescriptor | null;
	/** The declared-only descriptor the runtime part was split from. */
	declaredDescriptor: WidgetPolicyDescriptor | null;
	/** For the capability dialog: the policy allowing grants (declared, plus runtime when included). */
	subject: MicroWidgetPolicySubject;
	declared: WidgetDeclaredPart & { covered: boolean };
	runtime: {
		/** Runtime sources this dialog asks for. */
		pending: WidgetRuntimeSourceEntry[];
		/** Runtime sources of the descriptor that are already allowed. */
		allowed: WidgetRuntimeSourceEntry[];
		level: WidgetSourceLevel | null;
		slots: string[];
	};
	newSources: string[];
	raisedSources: string[];
	newCapabilities: MicroWidgetCapability[];
	/** Only in mount mode with a declared part also pending. */
	hasRuntimeCheckbox: boolean;
	/** Checked unless the runtime part lets anyone receive what the widget sends (`broad`). */
	includeRuntimeDefault: boolean;
	includeRuntime: boolean;
	/** A newer extraction exists; `showNewerPrompt` swaps it in. */
	newerAvailable: boolean;
	canAllowForProject: boolean;
	canStopAsking: boolean;
	/** Classification level of the whole network, when the backend classified it. */
	level: WidgetSourceLevel | null;
}

/** Non-modal banner while the widget runs: new runtime sources wait for review. */
export interface MicroWidgetRuntimeRequest {
	sources: string[];
	count: number;
	level: WidgetSourceLevel;
}

export type MicroWidgetGrantNotice =
	| { kind: "invalid"; reason: string | null }
	| { kind: "baseline" }
	| { kind: "unavailable" }
	| { kind: "runtime-unavailable" }
	| {
			kind: "runtime-skipped";
			rejected: WidgetRuntimeRejection[];
			issues: RuntimeValueIssue[];
			invalidReason: string | null;
	  };

export type MicroWidgetGrantNoticeKind = MicroWidgetGrantNotice["kind"];

/** `promptKey`: the `key` of the rendered prompt; the decision is ignored once the prompt shows something else. */
export interface MicroWidgetGrantActions {
	allowOnce: (promptKey?: string) => void;
	allowForProject: (promptKey?: string) => void;
	/** Runtime-only requests session-block their new sources; anything else blocks the widget. */
	dontAllow: (promptKey?: string) => void;
	/** Don't allow, and never ask about runtime sources again (project, else session). */
	stopAsking: (promptKey?: string) => void;
	/** Blocked card: prompt again. Runtime banner: open the runtime dialog. */
	review: () => void;
	/** Blocked card: run the baseline frame. */
	runRestricted: () => void;
	setIncludeRuntime: (include: boolean) => void;
	showNewerPrompt: () => void;
	dismissRuntimeRequest: () => void;
	dismissNotice: (kind: MicroWidgetGrantNoticeKind) => void;
	/** Call from the iframe `load` handler: re-mints a grant that is past its deadline. */
	onFrameLoad: () => void;
	/** Call when the frame's widget said `hello`; later document swaps are rate limited. */
	onFrameHello: () => void;
}

export interface MicroWidgetGrant extends MicroWidgetGrantActions {
	state: MicroWidgetGrantState;
	consent: MicroWidgetConsentStatus;
	prompt: MicroWidgetConsentPrompt | null;
	runtimeRequest: MicroWidgetRuntimeRequest | null;
	/** Highest priority first; dismissed kinds are left out. */
	notices: MicroWidgetGrantNotice[];
}

interface CachedGrant {
	grant: string | null;
	runtime: string | null;
	deadline: number;
	packageId: string;
	widgetId: string;
}

const grantCache = new Map<string, CachedGrant>();
/** Revision of the latest forget per `[packageId]` or `[packageId, widgetId]`. */
const forgottenGrants = new Map<string, number>();
const forgetListeners = new Set<() => void>();
let forgetRevision = 0;

function forgottenKey(packageId: string, widgetId?: string): string {
	return JSON.stringify(
		widgetId === undefined ? [packageId] : [packageId, widgetId],
	);
}

function readForgottenRevision(packageId: string, widgetId: string): number {
	return Math.max(
		forgottenGrants.get(forgottenKey(packageId)) ?? 0,
		forgottenGrants.get(forgottenKey(packageId, widgetId)) ?? 0,
	);
}

function subscribeForgottenGrants(listener: () => void): () => void {
	forgetListeners.add(listener);
	return () => {
		forgetListeners.delete(listener);
	};
}

/**
 * Drops cached grants after the backend revoked them (uninstall, revoke), so
 * mounted widgets mint again instead of loading an id that now serves baseline.
 * Without `widgetId` every widget of the package is forgotten.
 */
export function forgetMicroWidgetGrants(
	packageId: string,
	widgetId?: string,
): void {
	for (const [key, entry] of grantCache) {
		if (
			entry.packageId === packageId &&
			(widgetId === undefined || entry.widgetId === widgetId)
		) {
			grantCache.delete(key);
		}
	}
	forgottenGrants.set(forgottenKey(packageId, widgetId), ++forgetRevision);
	for (const listener of forgetListeners) listener();
}

function readCachedGrant(key: string, now: number): CachedGrant | null {
	const entry = grantCache.get(key);
	if (!entry) return null;
	grantCache.delete(key);
	if (now >= entry.deadline) return null;
	grantCache.set(key, entry);
	return entry;
}

function writeCachedGrant(key: string, entry: CachedGrant, now: number): void {
	for (const [cached, value] of grantCache) {
		if (value.deadline <= now) grantCache.delete(cached);
	}
	if (entry.deadline <= now) return;
	grantCache.delete(key);
	grantCache.set(key, entry);
	while (grantCache.size > MICRO_WIDGET_GRANT_CACHE_LIMIT) {
		const oldest = grantCache.keys().next().value;
		if (oldest === undefined) break;
		grantCache.delete(oldest);
	}
}

export function microWidgetGrantCacheSizeForTests(): number {
	return grantCache.size;
}

export function resetMicroWidgetGrantCacheForTests(): void {
	grantCache.clear();
	forgottenGrants.clear();
}

function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	if (typeof error === "object" && error !== null) {
		const { message, error: inner } = error as Record<string, unknown>;
		if (typeof message === "string") return message;
		if (typeof inner === "string") return inner;
	}
	return String(error);
}

/** Registry methods, bound; null when the backend predates widget grants or is a placeholder that throws. */
function registryMethod<K extends "describeWidgetPolicy" | "mintWidgetGrant">(
	registry: IRegistryState | null | undefined,
	name: K,
): NonNullable<IRegistryState[K]> | null {
	try {
		const method = registry?.[name];
		return typeof method === "function"
			? (method.bind(registry) as NonNullable<IRegistryState[K]>)
			: null;
	} catch {
		return null;
	}
}

/**
 * A page contract without `csp` (v1) may use the pre-grant URL when the backend
 * cannot describe the widget: old servers keep today's behavior and new ones
 * serve that URL at baseline. Anything else needs a server that enforces grants.
 */
export function allowsLegacyMicroWidgetFrame(
	contract: WidgetContract | null | undefined,
): boolean {
	if (typeof contract !== "object" || contract === null) return true;
	const { contractVersion, csp } = contract as unknown as Record<
		string,
		unknown
	>;
	const v1 =
		contractVersion === undefined ||
		contractVersion === null ||
		contractVersion === 1;
	return v1 && (csp === undefined || csp === null);
}

export interface MicroWidgetFrameLocation {
	desktop: boolean;
	useHttpBridge: boolean;
	/** Resolves an API path on web; null while the profile that picks the API origin loads. */
	apiUrl: ((path: string) => string) | null;
}

/** Throws when a grant or runtime component is malformed, so a bad backend answer never becomes a URL. */
export function microWidgetFrameSrc(
	frame: MicroWidgetFrameMount,
	location: MicroWidgetFrameLocation,
): string | null {
	const allowDownloads = frame.policy.downloads === true;
	if (location.desktop) {
		if (!frame.bundleHash) return null;
		return frame.kind === "legacy"
			? buildDesktopMicroWidgetSrc({
					packageId: frame.packageId,
					bundleHash: frame.bundleHash,
					widgetId: frame.widgetId,
					useHttpBridge: location.useHttpBridge,
					allowDownloads,
				})
			: buildDesktopMicroWidgetFrameSrc({
					packageId: frame.packageId,
					bundleHash: frame.bundleHash,
					widgetId: frame.widgetId,
					grant: frame.grant,
					useHttpBridge: location.useHttpBridge,
				});
	}
	if (!location.apiUrl) return null;
	return location.apiUrl(
		frame.kind === "legacy"
			? buildWebMicroWidgetPath(
					frame.packageId,
					frame.packageVersion,
					frame.widgetId,
					allowDownloads,
				)
			: buildWebMicroWidgetFramePath({
					packageId: frame.packageId,
					packageVersion: frame.packageVersion,
					widgetId: frame.widgetId,
					grant: frame.grant,
					runtime: frame.runtime,
				}),
	);
}

export interface MicroWidgetGrantClock {
	/** Monotonic milliseconds. */
	now: () => number;
	setTimeout: (callback: () => void, ms: number) => unknown;
	clearTimeout: (handle: unknown) => void;
}

const DEFAULT_CLOCK: MicroWidgetGrantClock = {
	now: () =>
		typeof performance !== "undefined" ? performance.now() : Date.now(),
	setTimeout: (callback, ms) => setTimeout(callback, ms),
	clearTimeout: (handle) =>
		clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** Everything the controller reads from the component. */
export interface MicroWidgetGrantControllerInputs {
	registry: IRegistryState | null;
	active: boolean;
	packageId: string;
	packageVersion: string;
	bundleHash: string | null;
	widgetId: string;
	preview: boolean;
	appId: string | null;
	legacyAllowed: boolean;
	legacyPolicy: WidgetPolicy;
	/** `publicWidgetProps` of the instance props, as plain JSON data. */
	props: Readonly<Record<string, unknown>>;
	propsKey: string;
}

type DescribeOutcome =
	| { kind: "described"; descriptor: WidgetPolicyDescriptor }
	| { kind: "fallback"; detail: string | null };

interface Candidate {
	descriptor: WidgetPolicyDescriptor;
	/** Exactly what describe was asked; every mint of this candidate sends it again. */
	request: WidgetRuntimeSourceRequest[];
	key: string;
	split: WidgetDescriptorSplit;
}

type MintState = (
	| { kind: "minting" }
	| {
			kind: "granted";
			grant: string | null;
			runtime: string | null;
			deadline: number;
	  }
	| { kind: "unavailable" }
	| { kind: "failed"; detail: string }
) & { forgotten: number };

interface Displayed {
	candidate: Candidate;
	mintKey: string;
	frameKey: string;
	grant: string | null;
	runtime: string | null;
	deadline: number;
}

type RuntimeStage = "idle" | "grace" | "describing" | "done";
type RuntimeDescribeMode = "mount" | "update" | "retry";

interface Backoff {
	until: number;
	delay: number;
}

function nextBackoff(previous: Backoff | null, now: number): Backoff {
	const delay = previous
		? Math.min(previous.delay * 2, MICRO_WIDGET_RUNTIME_MAX_BACKOFF_MS)
		: MICRO_WIDGET_RUNTIME_BACKOFF_MS;
	return { until: now + delay, delay };
}

/** Registry implementations outside this package may omit fields the parser always fills. */
function normalizeDescriptor(
	descriptor: WidgetPolicyDescriptor,
): WidgetPolicyDescriptor {
	return Array.isArray(descriptor.networkInputs)
		? descriptor
		: { ...descriptor, networkInputs: [] };
}

function consentRequestOf(candidate: Candidate): MicroWidgetConsentRequest {
	return {
		policy: candidate.split.declared.policy,
		levels: candidate.split.declared.levels,
		runtime: candidate.split.runtime,
	};
}

function legacyPromptKey(policy: WidgetPolicy): string {
	return JSON.stringify(["legacy", policyCapabilities(policy)]);
}

function runtimePairs(candidate: Candidate | null): Set<string> {
	return new Set(
		(candidate?.split.runtime ?? []).map(
			(entry) => `${entry.directive} ${entry.source}`,
		),
	);
}

function distinctSources(
	entries: readonly WidgetRuntimeSourceEntry[],
): string[] {
	return [...new Set(entries.map((entry) => entry.source))].sort();
}

const INITIAL_SNAPSHOT_STATE: Pick<
	MicroWidgetGrant,
	"state" | "consent" | "prompt" | "runtimeRequest" | "notices"
> = {
	state: { status: "describing" },
	consent: "pending",
	prompt: null,
	runtimeRequest: null,
	notices: [],
};

/**
 * Mount flow of one package widget instance (§6.3, §14.4.10): describe the
 * declared policy, extract runtime sources from the props the widget
 * receives, describe them, check consent against the split descriptor, mint
 * with exactly the approved runtime sources and hand back the frame to mount.
 * While a frame runs, later props are re-extracted (debounced); covered
 * additions remount the frame (rate limited once the widget said hello) and
 * uncovered ones surface as a banner. Async results that arrive after their
 * inputs changed are dropped through generation counters.
 */
export class MicroWidgetGrantController {
	private inputs: MicroWidgetGrantControllerInputs | null = null;
	private identity: string | null = null;
	private generation = 0;
	private active = false;
	private detached = false;
	private listeners = new Set<() => void>();
	private unsubscribers: (() => void)[] = [];
	private snapshot: MicroWidgetGrant;

	private d0: DescribeOutcome | null = null;
	private d0Pending = false;
	private staleDigest: string | null = null;

	private stage: RuntimeStage = "idle";
	private initialPropsKey: string | null = null;
	private graceDone = false;
	private dirty = false;
	private runtimeGeneration = 0;
	private runtimeDisabled = false;
	private runtimeConflicts = 0;
	private rejected = new Set<string>();
	private rejections = new Map<string, WidgetRuntimeRejection>();
	private issues: RuntimeValueIssue[] = [];
	private runtimeInvalid: string | null = null;
	private runtimeUnavailable = false;
	private backoff: Backoff | null = null;

	private candidate: Candidate | null = null;
	private frozen: Candidate | null = null;
	private reviewing = false;
	private includeRuntime: boolean | null = null;

	private mints = new Map<string, MintState>();
	/** Mint key the backend answered 409 for; not minted again until the retry describe settles. */
	private conflictedMint: string | null = null;
	private forgotten = 0;
	private displayed: Displayed | null = null;
	private queued: Displayed | null = null;
	private helloSeen = false;
	private lastSwapAt = Number.NEGATIVE_INFINITY;
	private lastRemint: number | null = null;
	private baselineRequested = false;
	private dismissedNotices = new Set<MicroWidgetGrantNoticeKind>();
	private bannerDismissal: (Backoff & { sources: Set<string> }) | null = null;

	private graceTimer: unknown = null;
	private debounceTimer: unknown = null;
	private burstStart: number | null = null;
	private swapTimer: unknown = null;
	private bannerTimer: unknown = null;

	readonly actions: MicroWidgetGrantActions;

	constructor(private readonly clock: MicroWidgetGrantClock = DEFAULT_CLOCK) {
		this.actions = {
			allowOnce: (promptKey) => this.allow("session", promptKey),
			allowForProject: (promptKey) => this.allow("app", promptKey),
			dontAllow: (promptKey) => this.dontAllow(false, promptKey),
			stopAsking: (promptKey) => this.dontAllow(true, promptKey),
			review: () => this.review(),
			runRestricted: () => {
				this.baselineRequested = true;
				this.reconcile();
			},
			setIncludeRuntime: (include) => {
				this.includeRuntime = include;
				this.emit();
			},
			showNewerPrompt: () => {
				if (!this.candidate || !this.frozen) return;
				this.frozen = this.candidate;
				this.includeRuntime = null;
				this.emit();
			},
			dismissRuntimeRequest: () => this.dismissRuntimeRequest(),
			dismissNotice: (kind) => {
				this.dismissedNotices.add(kind);
				this.emit();
			},
			onFrameLoad: () => this.onFrameLoad(),
			onFrameHello: () => {
				this.helloSeen = true;
			},
		};
		this.snapshot = { ...INITIAL_SNAPSHOT_STATE, ...this.actions };
	}

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	getSnapshot = (): MicroWidgetGrant => this.snapshot;

	/** Starts listening to consent and grant revocations. Pair with `deactivate`. */
	activate(): void {
		if (this.active) return;
		this.active = true;
		this.unsubscribers = [
			subscribeMicroWidgetConsent(() => this.reconcile()),
			subscribeForgottenGrants(() => this.onForgotten()),
		];
		if (this.detached && this.inputs) {
			this.detached = false;
			this.reset(this.identityOf(this.inputs));
		}
		this.reconcile();
	}

	/** Stops timers and listeners and drops in-flight work; `activate` starts over. */
	deactivate(): void {
		if (!this.active) return;
		this.active = false;
		this.detached = true;
		for (const unsubscribe of this.unsubscribers) unsubscribe();
		this.unsubscribers = [];
		this.reset(null);
	}

	setInputs(inputs: MicroWidgetGrantControllerInputs): void {
		const previous = this.inputs;
		const identity = this.identityOf(inputs);
		this.inputs = inputs;
		if (
			!previous ||
			identity !== this.identity ||
			previous.registry !== inputs.registry
		) {
			this.reset(identity);
			this.reconcile();
			return;
		}
		if (previous.propsKey !== inputs.propsKey) this.onPropsChanged();
		this.reconcile();
	}

	private identityOf(inputs: MicroWidgetGrantControllerInputs): string | null {
		return inputs.active
			? JSON.stringify([
					inputs.packageId,
					inputs.packageVersion,
					inputs.bundleHash,
					inputs.widgetId,
					inputs.preview,
					inputs.appId,
					inputs.legacyAllowed,
				])
			: null;
	}

	private clearTimer(handle: unknown): null {
		if (handle !== null) this.clock.clearTimeout(handle);
		return null;
	}

	private reset(identity: string | null): void {
		this.identity = identity;
		this.generation++;
		this.graceTimer = this.clearTimer(this.graceTimer);
		this.debounceTimer = this.clearTimer(this.debounceTimer);
		this.swapTimer = this.clearTimer(this.swapTimer);
		this.bannerTimer = this.clearTimer(this.bannerTimer);
		this.burstStart = null;
		this.d0 = null;
		this.d0Pending = false;
		this.staleDigest = null;
		this.initialPropsKey = this.inputs?.propsKey ?? null;
		this.graceDone = false;
		this.resetRuntime();
		this.runtimeDisabled = false;
		this.runtimeConflicts = 0;
		this.rejected = new Set();
		this.rejections = new Map();
		this.issues = [];
		this.runtimeInvalid = null;
		this.runtimeUnavailable = false;
		this.backoff = null;
		this.mints = new Map();
		this.forgotten = this.inputs
			? readForgottenRevision(this.inputs.packageId, this.inputs.widgetId)
			: 0;
		this.displayed = null;
		this.queued = null;
		this.helloSeen = false;
		this.lastSwapAt = Number.NEGATIVE_INFINITY;
		this.lastRemint = null;
		this.baselineRequested = false;
		this.dismissedNotices = new Set();
		this.bannerDismissal = null;
	}

	private resetRuntime(): void {
		this.runtimeGeneration++;
		this.stage = "idle";
		this.dirty = false;
		this.conflictedMint = null;
		this.candidate = null;
		this.frozen = null;
		this.reviewing = false;
		this.includeRuntime = null;
		this.graceTimer = this.clearTimer(this.graceTimer);
	}

	private declared(): WidgetPolicyDescriptor | null {
		return this.d0?.kind === "described" ? this.d0.descriptor : null;
	}

	private targetFor(descriptor: {
		source: string;
		packageId: string;
		widgetId: string;
	}): MicroWidgetConsentTarget {
		return {
			source: descriptor.source,
			appId: this.inputs?.appId ?? null,
			packageId: descriptor.packageId,
			widgetId: descriptor.widgetId,
		};
	}

	private legacyTarget(): MicroWidgetConsentTarget | null {
		const inputs = this.inputs;
		return inputs
			? {
					source: WIDGET_POLICY_SOURCE_ANY_REGISTRY,
					appId: inputs.appId,
					packageId: inputs.packageId,
					widgetId: inputs.widgetId,
				}
			: null;
	}

	private candidateFor(
		descriptor: WidgetPolicyDescriptor,
		request: WidgetRuntimeSourceRequest[],
	): Candidate {
		const declared = this.declared();
		return {
			descriptor,
			request,
			key: JSON.stringify([
				descriptor.policyDigest,
				descriptor.runtime?.runtimeDigest ?? null,
				widgetRuntimeRequestKey(request),
			]),
			split: splitWidgetPolicyDescriptor(
				descriptor,
				declared?.policy ?? descriptor.policy,
			),
		};
	}

	private declaredCandidate(): Candidate | null {
		const declared = this.declared();
		return declared ? this.candidateFor(declared, NO_RUNTIME) : null;
	}

	private evaluate(candidate: Candidate): MicroWidgetConsentEvaluation {
		return evaluateMicroWidgetConsent(
			this.targetFor(candidate.descriptor),
			consentRequestOf(candidate),
		);
	}

	private mintKey(candidate: Candidate): string {
		const { descriptor } = candidate;
		return JSON.stringify([
			descriptor.source,
			this.inputs?.appId ?? null,
			descriptor.packageId,
			descriptor.packageVersion ?? this.inputs?.packageVersion ?? null,
			descriptor.bundleHash,
			descriptor.widgetId,
			descriptor.preview,
			descriptor.policyDigest,
			descriptor.runtime?.runtimeDigest ?? null,
		]);
	}

	private isCurrent(generation: number): boolean {
		return this.active && this.generation === generation;
	}

	private reconcile(): void {
		const inputs = this.inputs;
		if (!this.active || !inputs || !inputs.active) {
			this.emit();
			return;
		}
		if (!this.d0 && !this.d0Pending) this.describeDeclared();
		const declared = this.declared();
		if (
			declared &&
			declared.status === "ok" &&
			declared.policyDigest !== this.staleDigest
		) {
			this.dropRefusedRuntime(declared);
			if (this.stage === "idle") this.startRuntime(declared);
			this.reconcileCandidate();
		}
		this.emit();
	}

	/**
	 * Instances of a widget share one dialog. When one of them refused runtime
	 * sources (Don't allow, Stop asking, an unchecked runtime box), the others
	 * stop asking for or minting them: back to the running frame's sources,
	 * or a fresh extraction that skips them.
	 */
	private dropRefusedRuntime(declared: WidgetPolicyDescriptor): void {
		const runningKey = this.displayed?.candidate.key;
		const asking = [this.frozen, this.candidate].filter(
			(candidate): candidate is Candidate =>
				candidate !== null &&
				candidate.key !== runningKey &&
				candidate.request.length > 0,
		);
		if (asking.length === 0) return;
		const blocks = readMicroWidgetRuntimeBlocks(this.targetFor(declared));
		const refused =
			asking.some((candidate) =>
				widgetRuntimeRequestSources(candidate.request).some((source) =>
					blocks.has(source),
				),
			) || this.muted(declared);
		if (!refused) return;
		if (!this.displayed) {
			this.restartRuntime();
			return;
		}
		this.candidate = this.displayed.candidate;
		this.frozen = null;
		this.reviewing = false;
		this.includeRuntime = null;
	}

	private describeDeclared(): void {
		const inputs = this.inputs;
		if (!inputs) return;
		const describe = registryMethod(inputs.registry, "describeWidgetPolicy");
		if (!describe) {
			this.d0 = { kind: "fallback", detail: null };
			return;
		}
		this.d0Pending = true;
		const generation = this.generation;
		Promise.resolve()
			.then(() =>
				describe({
					packageId: inputs.packageId,
					packageVersion: inputs.packageVersion,
					bundleHash: inputs.bundleHash,
					widgetId: inputs.widgetId,
					preview: inputs.preview,
				}),
			)
			.then(
				(descriptor) => {
					if (!this.isCurrent(generation)) return;
					this.d0Pending = false;
					this.d0 = {
						kind: "described",
						descriptor: normalizeDescriptor(descriptor),
					};
					this.reconcile();
				},
				(error) => {
					if (!this.isCurrent(generation)) return;
					this.d0Pending = false;
					this.d0 = { kind: "fallback", detail: errorMessage(error) };
					this.reconcile();
				},
			);
	}

	/** Declared digest rejected at mint: describe again; the same digest twice is an error. */
	private redescribeDeclared(rejectedDigest: string): void {
		this.staleDigest = rejectedDigest;
		this.d0 = null;
		this.d0Pending = false;
		this.resetRuntime();
		this.mints = new Map();
		this.displayed = null;
		this.queued = null;
		this.swapTimer = this.clearTimer(this.swapTimer);
	}

	private muted(declared: WidgetPolicyDescriptor): boolean {
		return evaluateMicroWidgetConsent(this.targetFor(declared), {}).muted;
	}

	private runtimeEligible(declared: WidgetPolicyDescriptor): boolean {
		const inputs = this.inputs;
		return (
			inputs !== null &&
			declared.status === "ok" &&
			!declared.preview &&
			declared.networkInputs.length > 0 &&
			!this.runtimeDisabled &&
			declared.engine?.runtimeSources !== false &&
			registryMethod(inputs.registry, "describeWidgetPolicy") !== null &&
			(this.backoff === null || this.clock.now() >= this.backoff.until) &&
			!this.muted(declared)
		);
	}

	private extract(declared: WidgetPolicyDescriptor) {
		const target = this.targetFor(declared);
		const excluded = new Set([
			...readMicroWidgetRuntimeBlocks(target),
			...this.rejected,
		]);
		const extraction = extractRuntimeSources(
			declared.networkInputs,
			declared.platformStorage ?? [],
			this.inputs?.appId ?? null,
			this.inputs?.props ?? {},
			excluded,
		);
		this.issues = extraction.issues;
		return extraction;
	}

	private startRuntime(declared: WidgetPolicyDescriptor): void {
		if (!this.runtimeEligible(declared)) {
			if (
				declared.engine?.runtimeSources === false &&
				this.extract(declared).request.length > 0
			) {
				this.runtimeUnavailable = true;
			}
			this.stage = "done";
			this.candidate = this.declaredCandidate();
			return;
		}
		const extraction = this.extract(declared);
		if (extraction.request.length === 0 && !this.graceDone) {
			this.stage = "grace";
			this.graceTimer = this.clock.setTimeout(() => {
				this.graceTimer = null;
				if (this.stage !== "grace") return;
				this.finishMount();
				this.reconcile();
			}, MICRO_WIDGET_RUNTIME_GRACE_MS);
			return;
		}
		this.finishMount(extraction.request);
	}

	private finishMount(request?: WidgetRuntimeSourceRequest[]): void {
		const declared = this.declared();
		if (!declared) return;
		this.graceTimer = this.clearTimer(this.graceTimer);
		this.graceDone = true;
		const next = request ?? this.extract(declared).request;
		if (next.length === 0) {
			this.stage = "done";
			this.candidate = this.declaredCandidate();
			return;
		}
		this.stage = "describing";
		this.describeRuntime(next, "mount");
	}

	/** Starts the mount stage over, e.g. after runtime sources were refused. */
	private restartRuntime(): void {
		this.resetRuntime();
		this.graceDone = true;
	}

	private onPropsChanged(): void {
		if (this.inputs?.propsKey !== this.initialPropsKey) this.graceDone = true;
		switch (this.stage) {
			case "grace":
				this.finishMount();
				break;
			case "describing":
				this.dirty = true;
				break;
			case "done":
				this.scheduleUpdate();
				break;
			case "idle":
				break;
		}
	}

	private scheduleUpdate(): void {
		const now = this.clock.now();
		this.burstStart ??= now;
		this.debounceTimer = this.clearTimer(this.debounceTimer);
		const delay = Math.max(
			0,
			Math.min(
				MICRO_WIDGET_RUNTIME_DEBOUNCE_MS,
				this.burstStart + MICRO_WIDGET_RUNTIME_MAX_WAIT_MS - now,
			),
		);
		this.debounceTimer = this.clock.setTimeout(() => {
			this.debounceTimer = null;
			this.burstStart = null;
			this.runUpdate();
			this.reconcile();
		}, delay);
	}

	private runUpdate(): void {
		const declared = this.declared();
		if (!declared) return;
		if (this.stage === "describing" || this.conflictedMint !== null) {
			this.dirty = true;
			return;
		}
		if (this.stage !== "done" || !this.runtimeEligible(declared)) return;
		const extraction = this.extract(declared);
		const base = this.displayed?.candidate ?? this.candidate;
		const mounted = (base?.request ?? []).map(({ slot, sources }) => ({
			slot,
			sources: sources.filter((source) => !this.rejected.has(source)),
		}));
		const mountedSources = new Set(widgetRuntimeRequestSources(mounted));
		const need = widgetRuntimeRequestSources(extraction.request).filter(
			(source) => !mountedSources.has(source),
		);
		if (need.length === 0) return;
		const union = canonicalWidgetRuntimeRequest([
			...mounted,
			...extraction.request,
		]);
		const next =
			widgetRuntimeRequestSources(union).length <=
			RUNTIME_EXTRACTION_LIMITS.maxSourcesTotal
				? union
				: extraction.request;
		if (
			this.candidate &&
			widgetRuntimeRequestKey(next) ===
				widgetRuntimeRequestKey(this.candidate.request)
		) {
			return;
		}
		this.describeRuntime(next, "update");
	}

	private describeRuntime(
		request: WidgetRuntimeSourceRequest[],
		mode: RuntimeDescribeMode,
	): void {
		const inputs = this.inputs;
		const describe = inputs
			? registryMethod(inputs.registry, "describeWidgetPolicy")
			: null;
		if (!inputs || !describe) {
			this.disableRuntime(mode !== "update");
			return;
		}
		const generation = this.generation;
		const runtimeGeneration = ++this.runtimeGeneration;
		const current = () =>
			this.isCurrent(generation) &&
			this.runtimeGeneration === runtimeGeneration;
		Promise.resolve()
			.then(() =>
				describe({
					packageId: inputs.packageId,
					packageVersion: inputs.packageVersion,
					bundleHash: inputs.bundleHash,
					widgetId: inputs.widgetId,
					preview: inputs.preview,
					appId: inputs.appId,
					runtimeSources: request,
				}),
			)
			.then(
				(descriptor) => {
					if (!current()) return;
					this.conflictedMint = null;
					this.onRuntimeDescribed(
						normalizeDescriptor(descriptor),
						request,
						mode,
					);
					this.reconcile();
				},
				(error) => {
					if (!current()) return;
					this.conflictedMint = null;
					this.onRuntimeFailed(error, mode);
					this.reconcile();
				},
			);
	}

	private settleRuntime(next: Candidate | null, mode: RuntimeDescribeMode) {
		if (mode === "update") {
			if (!next) return;
			const mounted = runtimePairs(this.displayed?.candidate ?? this.candidate);
			const accepted = [...runtimePairs(next)];
			if (accepted.every((pair) => mounted.has(pair))) return;
			this.candidate = next;
			return;
		}
		this.candidate = next ?? this.declaredCandidate();
		this.stage = "done";
		if (this.dirty) {
			this.dirty = false;
			this.scheduleUpdate();
		}
	}

	private onRuntimeDescribed(
		descriptor: WidgetPolicyDescriptor,
		request: WidgetRuntimeSourceRequest[],
		mode: RuntimeDescribeMode,
	): void {
		const runtime = descriptor.runtime;
		if (descriptor.status !== "ok") {
			this.settleRuntime(this.candidateFor(descriptor, NO_RUNTIME), mode);
			return;
		}
		if (!runtime) {
			this.runtimeUnavailable = true;
			this.disableRuntime(mode !== "update");
			return;
		}
		for (const rejection of runtime.rejected) {
			this.rejected.add(rejection.source);
			this.rejections.set(
				JSON.stringify([rejection.slot, rejection.source]),
				rejection,
			);
		}
		switch (runtime.status) {
			case "unavailable":
				this.backoff = nextBackoff(this.backoff, this.clock.now());
				this.runtimeUnavailable = true;
				this.settleRuntime(null, mode);
				return;
			case "invalid":
				this.runtimeInvalid = runtime.invalidReason ?? runtime.status;
				this.settleRuntime(null, mode);
				return;
			case "none":
				this.settleRuntime(null, mode);
				return;
			case "ok":
				this.backoff = null;
				this.runtimeInvalid = null;
				this.settleRuntime(
					runtime.runtimeDigest ? this.candidateFor(descriptor, request) : null,
					mode,
				);
				return;
		}
	}

	private onRuntimeFailed(error: unknown, mode: RuntimeDescribeMode): void {
		if (isWidgetRuntimeDescribeUnsupportedError(error)) {
			this.runtimeUnavailable = true;
			this.disableRuntime(mode !== "update");
			return;
		}
		if (widgetRuntimeSourcesErrorCode(error)) {
			console.warn(
				`[MicroWidget] runtime sources refused for ${this.inputs?.packageId}/${this.inputs?.widgetId}: ${errorMessage(error)}`,
			);
			this.disableRuntime(mode !== "update");
			return;
		}
		this.backoff = nextBackoff(this.backoff, this.clock.now());
		this.runtimeUnavailable = true;
		this.settleRuntime(null, mode);
	}

	/** Declared-only for the rest of this mount. */
	private disableRuntime(revert: boolean): void {
		this.runtimeDisabled = true;
		this.runtimeGeneration++;
		this.conflictedMint = null;
		this.debounceTimer = this.clearTimer(this.debounceTimer);
		this.burstStart = null;
		if (this.stage !== "done" || revert) {
			this.stage = "done";
			this.dirty = false;
			this.candidate = this.declaredCandidate();
		}
	}

	private reconcileCandidate(): void {
		if (
			this.displayed &&
			this.evaluate(this.displayed.candidate).status !== "granted"
		) {
			this.displayed = null;
			this.queued = null;
			this.swapTimer = this.clearTimer(this.swapTimer);
		}
		const candidate = this.candidate;
		if (!candidate) return;
		if (
			candidate.descriptor.status !== "ok" ||
			isEmptyPolicy(candidate.descriptor.policy)
		) {
			this.displayed = null;
			this.frozen = null;
			return;
		}
		const evaluation = this.evaluate(candidate);
		if (evaluation.status === "granted") {
			this.frozen = null;
			this.reviewing = false;
			if (this.conflictedMint === this.mintKey(candidate)) return;
			const mint = this.ensureMint(candidate);
			if (mint.kind === "granted") this.offerFrame(candidate, mint);
			return;
		}
		if (evaluation.status === "blocked") {
			this.frozen = null;
			this.reviewing = false;
			return;
		}
		if (this.frozen && this.evaluate(this.frozen).status === "granted") {
			this.frozen = null;
			this.includeRuntime = null;
		}
		const running = this.displayed !== null && evaluation.declaredCovered;
		if (running && !this.reviewing) {
			this.frozen = null;
			return;
		}
		this.frozen ??= candidate;
	}

	private ensureMint(candidate: Candidate): MintState {
		const inputs = this.inputs as MicroWidgetGrantControllerInputs;
		const { descriptor } = candidate;
		const key = this.mintKey(candidate);
		const forgotten = readForgottenRevision(
			descriptor.packageId,
			descriptor.widgetId,
		);
		const existing = this.mints.get(key);
		if (existing && existing.forgotten === forgotten) return existing;
		const now = this.clock.now();
		const cached = readCachedGrant(key, now);
		if (cached) {
			const state: MintState = {
				kind: "granted",
				grant: cached.grant,
				runtime: cached.runtime,
				deadline: cached.deadline,
				forgotten,
			};
			this.mints.set(key, state);
			return state;
		}
		const mintGrant = registryMethod(inputs.registry, "mintWidgetGrant");
		if (!mintGrant) {
			const state: MintState = { kind: "unavailable", forgotten };
			this.mints.set(key, state);
			return state;
		}
		const entry: MintState = { kind: "minting", forgotten };
		this.mints.set(key, entry);
		const withRuntime = descriptor.runtime?.runtimeDigest !== undefined;
		const target = this.targetFor(descriptor);
		Promise.resolve()
			.then(() =>
				mintGrant({
					packageId: descriptor.packageId,
					packageVersion: descriptor.packageVersion ?? inputs.packageVersion,
					bundleHash: descriptor.bundleHash,
					widgetId: descriptor.widgetId,
					preview: descriptor.preview,
					policyDigest: descriptor.policyDigest,
					...(withRuntime
						? { appId: inputs.appId, runtimeSources: candidate.request }
						: {}),
				}),
			)
			.then(
				(response) => {
					if (this.mints.get(key) !== entry) return;
					if (response.policyDigest !== descriptor.policyDigest) {
						this.onPolicyChanged(candidate, key);
						this.reconcile();
						return;
					}
					if (
						readForgottenRevision(descriptor.packageId, descriptor.widgetId) !==
						forgotten
					) {
						this.mints.delete(key);
						this.reconcile();
						return;
					}
					const deadline =
						now +
						response.expiresIn * 1000 -
						MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS;
					writeCachedGrant(
						key,
						{
							grant: response.grant,
							runtime: response.runtime ?? null,
							deadline,
							packageId: descriptor.packageId,
							widgetId: descriptor.widgetId,
						},
						this.clock.now(),
					);
					if (!withRuntime) this.staleDigest = null;
					this.mints.set(key, {
						kind: "granted",
						grant: response.grant,
						runtime: response.runtime ?? null,
						deadline,
						forgotten,
					});
					refreshMicroWidgetRuntimeConsent(target, candidate.split.runtime);
					this.reconcile();
				},
				(error) => {
					if (this.mints.get(key) !== entry) return;
					if (isPolicyChangedError(error)) {
						this.onPolicyChanged(candidate, key);
					} else if (widgetRuntimeSourcesErrorCode(error) && withRuntime) {
						this.mints.delete(key);
						this.disableRuntime(true);
					} else if (isWidgetGrantUnavailableError(error)) {
						this.mints.set(key, { kind: "unavailable", forgotten });
					} else {
						this.mints.set(key, {
							kind: "failed",
							detail: errorMessage(error),
							forgotten,
						});
					}
					this.reconcile();
				},
			);
		return entry;
	}

	/** 409 with runtime sources: describe them again once, then run declared-only. */
	private onPolicyChanged(candidate: Candidate, key: string): void {
		grantCache.delete(key);
		this.mints.delete(key);
		if (candidate.descriptor.runtime?.runtimeDigest !== undefined) {
			this.runtimeConflicts++;
			if (this.runtimeConflicts >= 2) {
				this.disableRuntime(true);
				return;
			}
			this.conflictedMint = key;
			this.describeRuntime(candidate.request, "retry");
			return;
		}
		this.redescribeDeclared(candidate.descriptor.policyDigest);
	}

	private offerFrame(
		candidate: Candidate,
		mint: Extract<MintState, { kind: "granted" }>,
	): void {
		const next: Displayed = {
			candidate,
			mintKey: this.mintKey(candidate),
			frameKey: JSON.stringify([mint.grant, mint.runtime]),
			grant: mint.grant,
			runtime: mint.runtime,
			deadline: mint.deadline,
		};
		const now = this.clock.now();
		if (!this.displayed || this.displayed.frameKey === next.frameKey) {
			if (!this.displayed) this.markSwap(now);
			this.displayed = next;
			return;
		}
		if (this.queued?.frameKey === next.frameKey) {
			this.queued = next;
			return;
		}
		if (
			!this.helloSeen ||
			now - this.lastSwapAt >= MICRO_WIDGET_SWAP_INTERVAL_MS
		) {
			this.displayed = next;
			this.queued = null;
			this.swapTimer = this.clearTimer(this.swapTimer);
			this.markSwap(now);
			return;
		}
		this.queued = next;
		if (this.swapTimer !== null) return;
		this.swapTimer = this.clock.setTimeout(
			() => {
				this.swapTimer = null;
				const queued = this.queued;
				this.queued = null;
				if (queued) {
					this.displayed = queued;
					this.markSwap(this.clock.now());
				}
				this.reconcile();
			},
			this.lastSwapAt + MICRO_WIDGET_SWAP_INTERVAL_MS - now,
		);
	}

	private markSwap(now: number): void {
		this.lastSwapAt = now;
		this.helloSeen = false;
	}

	private onForgotten(): void {
		const inputs = this.inputs;
		if (!inputs) return;
		const declared = this.declared();
		const revision = readForgottenRevision(
			declared?.packageId ?? inputs.packageId,
			declared?.widgetId ?? inputs.widgetId,
		);
		if (revision === this.forgotten) return;
		this.forgotten = revision;
		this.mints = new Map();
		this.displayed = null;
		this.queued = null;
		this.swapTimer = this.clearTimer(this.swapTimer);
		this.reconcile();
	}

	private onFrameLoad(): void {
		const shown = this.displayed;
		if (!shown || shown.grant === null) return;
		const now = this.clock.now();
		if (now < shown.deadline) return;
		if (
			this.lastRemint !== null &&
			now - this.lastRemint < MICRO_WIDGET_REMINT_INTERVAL_MS
		) {
			return;
		}
		this.lastRemint = now;
		grantCache.delete(shown.mintKey);
		this.mints.delete(shown.mintKey);
		this.reconcile();
	}

	private promptCandidate(): {
		candidate: Candidate;
		mode: MicroWidgetConsentPrompt["mode"];
	} | null {
		if (!this.frozen) return null;
		const running =
			this.displayed !== null &&
			this.candidate !== null &&
			this.evaluate(this.candidate).declaredCovered;
		return {
			candidate: this.frozen,
			mode: running && this.reviewing ? "runtime" : "mount",
		};
	}

	private legacyPromptMatches(promptKey: string | undefined): boolean {
		return (
			promptKey === undefined ||
			(this.inputs !== null &&
				promptKey === legacyPromptKey(this.inputs.legacyPolicy))
		);
	}

	/**
	 * Grants what the prompt showed. A project grant never persists a declared
	 * part the dialog hid, nor runtime sources it listed as already allowed.
	 */
	private allow(scope: MicroWidgetConsentScope, promptKey?: string): void {
		const inputs = this.inputs;
		if (!inputs) return;
		if (this.d0?.kind === "fallback") {
			if (!this.legacyPromptMatches(promptKey)) return;
			const target = this.legacyTarget();
			if (target) grantMicroWidgetConsent(target, inputs.legacyPolicy, scope);
			this.reconcile();
			return;
		}
		const current = this.promptCandidate();
		if (!current) return;
		const prompt = this.buildPrompt(current.candidate, current.mode);
		if (promptKey !== undefined && prompt.key !== promptKey) return;
		const target = this.targetFor(current.candidate.descriptor);
		const newer = this.candidate?.key !== current.candidate.key;
		const { declared, runtime } = current.candidate.split;
		this.frozen = null;
		this.reviewing = false;
		this.includeRuntime = null;
		this.bannerDismissal = null;
		this.candidate = prompt.includeRuntime
			? current.candidate
			: this.declaredCandidate();
		grantMicroWidgetConsent(
			target,
			{
				policy: declared.policy,
				levels: declared.levels,
				runtime: prompt.includeRuntime ? runtime : [],
				shown: {
					declared: !prompt.declared.covered,
					runtime: prompt.includeRuntime ? prompt.runtime.pending : [],
				},
			},
			scope,
		);
		if (!prompt.includeRuntime) {
			blockMicroWidgetRuntimeSources(
				target,
				distinctSources(prompt.runtime.pending),
			);
		}
		if (newer) this.scheduleUpdate();
		this.reconcile();
	}

	private dontAllow(mute: boolean, promptKey?: string): void {
		if (this.d0?.kind === "fallback") {
			if (!this.legacyPromptMatches(promptKey)) return;
			const target = this.legacyTarget();
			if (target) blockMicroWidgetConsent(target);
			this.reconcile();
			return;
		}
		const current = this.promptCandidate();
		const pendingCandidate = current?.candidate ?? this.candidate;
		if (!pendingCandidate) return;
		const prompt = this.buildPrompt(
			pendingCandidate,
			current?.mode ?? "runtime",
		);
		if (promptKey !== undefined && prompt.key !== promptKey) return;
		const target = this.targetFor(pendingCandidate.descriptor);
		this.frozen = null;
		this.reviewing = false;
		this.includeRuntime = null;
		this.bannerDismissal = null;
		if (prompt.declared.covered && prompt.runtime.pending.length > 0) {
			if (this.displayed) this.candidate = this.displayed.candidate;
			else this.restartRuntime();
			if (mute) muteMicroWidgetRuntime(target);
			blockMicroWidgetRuntimeSources(
				target,
				distinctSources(prompt.runtime.pending),
			);
		} else {
			this.baselineRequested = false;
			blockMicroWidgetConsent(target);
		}
		this.reconcile();
	}

	private review(): void {
		const state = this.snapshot.state;
		if (
			state.status === "blocked" ||
			(state.status === "ready" && state.frame.notice === "baseline")
		) {
			this.baselineRequested = false;
			const declared = this.declared();
			const target = declared ? this.targetFor(declared) : this.legacyTarget();
			if (target) reopenMicroWidgetConsent(target);
			this.reconcile();
			return;
		}
		if (!this.snapshot.runtimeRequest || !this.candidate) return;
		this.reviewing = true;
		this.frozen = this.candidate;
		this.includeRuntime = null;
		this.bannerDismissal = null;
		this.reconcile();
	}

	private dismissRuntimeRequest(): void {
		const request = this.snapshot.runtimeRequest;
		if (!request) return;
		const backoff = nextBackoff(this.bannerDismissal, this.clock.now());
		this.bannerDismissal = { ...backoff, sources: new Set(request.sources) };
		this.bannerTimer = this.clearTimer(this.bannerTimer);
		this.bannerTimer = this.clock.setTimeout(() => {
			this.bannerTimer = null;
			this.reconcile();
		}, backoff.delay);
		this.emit();
	}

	private buildPrompt(
		candidate: Candidate,
		mode: MicroWidgetConsentPrompt["mode"],
	): MicroWidgetConsentPrompt {
		const evaluation = this.evaluate(candidate);
		const { descriptor, split } = candidate;
		const pending = evaluation.uncoveredRuntime;
		const level = maxWidgetSourceLevel(pending.map((entry) => entry.level));
		const hasRuntimeCheckbox =
			mode === "mount" && !evaluation.declaredCovered && pending.length > 0;
		const includeRuntimeDefault = level !== "broad";
		const includeRuntime = hasRuntimeCheckbox
			? (this.includeRuntime ?? includeRuntimeDefault)
			: true;
		return {
			key: JSON.stringify([
				mode,
				candidate.key,
				evaluation.declaredCovered,
				pending.map((entry) => [entry.directive, entry.source, entry.level]),
			]),
			mode,
			descriptor,
			declaredDescriptor: this.declared(),
			subject: {
				source: descriptor.source,
				packageId: descriptor.packageId,
				widgetId: descriptor.widgetId,
				policy: includeRuntime ? descriptor.policy : split.declared.policy,
			},
			declared: { ...split.declared, covered: evaluation.declaredCovered },
			runtime: {
				pending,
				allowed: evaluation.coveredRuntime,
				level,
				slots: [
					...new Set(
						pending
							.map((entry) => entry.slot)
							.filter((slot): slot is string => slot !== null),
					),
				].sort(),
			},
			newSources: evaluation.newSources,
			raisedSources: evaluation.raisedSources,
			newCapabilities: evaluation.newCapabilities,
			hasRuntimeCheckbox,
			includeRuntimeDefault,
			includeRuntime,
			newerAvailable:
				this.candidate !== null && this.candidate.key !== candidate.key,
			canAllowForProject: Boolean(this.inputs?.appId),
			canStopAsking: evaluation.declaredCovered && pending.length > 0,
			level: descriptor.network?.level ?? null,
		};
	}

	private legacyPrompt(
		subject: MicroWidgetPolicySubject,
	): MicroWidgetConsentPrompt {
		return {
			key: legacyPromptKey(subject.policy),
			mode: "mount",
			descriptor: null,
			declaredDescriptor: null,
			subject,
			declared: { policy: subject.policy, levels: {}, covered: false },
			runtime: { pending: [], allowed: [], level: null, slots: [] },
			newSources: [],
			raisedSources: [],
			newCapabilities: policyCapabilities(subject.policy),
			hasRuntimeCheckbox: false,
			includeRuntimeDefault: true,
			includeRuntime: true,
			newerAvailable: false,
			canAllowForProject: Boolean(this.inputs?.appId),
			canStopAsking: false,
			level: null,
		};
	}

	private frameState(
		frame: Omit<MicroWidgetFrameMount, "kind"> & {
			kind?: MicroWidgetFrameMount["kind"];
		},
	): MicroWidgetGrantState {
		return { status: "ready", frame: { kind: "grant", ...frame } };
	}

	private baselineFrame(
		descriptor: WidgetPolicyDescriptor,
		notice: MicroWidgetFrameNotice | null,
	): MicroWidgetGrantState {
		return this.frameState({
			packageId: descriptor.packageId,
			packageVersion:
				descriptor.packageVersion ?? this.inputs?.packageVersion ?? "",
			bundleHash: descriptor.bundleHash,
			widgetId: descriptor.widgetId,
			grant: null,
			runtime: null,
			policy: BASELINE_POLICY,
			notice,
			invalidReason: descriptor.invalidReason ?? null,
		});
	}

	private displayedFrame(shown: Displayed): MicroWidgetGrantState {
		const { descriptor } = shown.candidate;
		return this.frameState({
			packageId: descriptor.packageId,
			packageVersion:
				descriptor.packageVersion ?? this.inputs?.packageVersion ?? "",
			bundleHash: descriptor.bundleHash,
			widgetId: descriptor.widgetId,
			grant: shown.grant,
			runtime: shown.runtime,
			policy: shown.grant === null ? BASELINE_POLICY : descriptor.policy,
			notice: null,
			invalidReason: null,
		});
	}

	private computeLegacy(
		inputs: MicroWidgetGrantControllerInputs,
		detail: string | null,
	): Pick<MicroWidgetGrant, "state" | "consent" | "prompt"> {
		if (!inputs.legacyAllowed) {
			return {
				state: { status: "unsupported", detail },
				consent: "pending",
				prompt: null,
			};
		}
		const subject: MicroWidgetPolicySubject = {
			source: WIDGET_POLICY_SOURCE_ANY_REGISTRY,
			packageId: inputs.packageId,
			widgetId: inputs.widgetId,
			policy: inputs.legacyPolicy,
		};
		const target = this.legacyTarget() as MicroWidgetConsentTarget;
		const consent = evaluateMicroWidgetConsent(
			target,
			inputs.legacyPolicy,
		).status;
		const frame = (policy: WidgetPolicy): MicroWidgetGrantState =>
			this.frameState({
				kind: "legacy",
				packageId: inputs.packageId,
				packageVersion: inputs.packageVersion,
				bundleHash: inputs.bundleHash,
				widgetId: inputs.widgetId,
				grant: null,
				runtime: null,
				policy,
				notice: null,
				invalidReason: null,
			});
		if (isEmptyPolicy(inputs.legacyPolicy)) {
			return {
				state: frame(BASELINE_POLICY),
				consent: "granted",
				prompt: null,
			};
		}
		if (consent === "granted") {
			return { state: frame(inputs.legacyPolicy), consent, prompt: null };
		}
		if (consent === "blocked") {
			return {
				state: { status: "blocked", subject, canRunBaseline: false },
				consent,
				prompt: null,
			};
		}
		return {
			state: { status: "pending", subject },
			consent,
			prompt: this.legacyPrompt(subject),
		};
	}

	private computeRuntimeRequest(
		evaluation: MicroWidgetConsentEvaluation,
	): MicroWidgetRuntimeRequest | null {
		const sources = distinctSources(evaluation.uncoveredRuntime);
		if (sources.length === 0) return null;
		const dismissal = this.bannerDismissal;
		if (
			dismissal &&
			this.clock.now() < dismissal.until &&
			sources.every((source) => dismissal.sources.has(source))
		) {
			return null;
		}
		return {
			sources,
			count: sources.length,
			level:
				maxWidgetSourceLevel(
					evaluation.uncoveredRuntime.map((entry) => entry.level),
				) ?? "broad",
		};
	}

	private computeDescribed(
		declared: WidgetPolicyDescriptor,
	): Pick<MicroWidgetGrant, "state" | "consent" | "prompt" | "runtimeRequest"> {
		const idle = { prompt: null, runtimeRequest: null };
		if (this.staleDigest === declared.policyDigest) {
			return {
				state: { status: "error", reason: "policy_unstable" },
				consent: "pending",
				...idle,
			};
		}
		if (declared.status === "invalid") {
			return {
				state: this.baselineFrame(declared, "invalid"),
				consent: "granted",
				...idle,
			};
		}
		const candidate = this.candidate;
		if (!candidate) {
			return { state: { status: "describing" }, consent: "pending", ...idle };
		}
		const { descriptor } = candidate;
		if (descriptor.status === "invalid") {
			return {
				state: this.baselineFrame(descriptor, "invalid"),
				consent: "granted",
				...idle,
			};
		}
		if (isEmptyPolicy(descriptor.policy)) {
			return {
				state: this.baselineFrame(descriptor, null),
				consent: "granted",
				...idle,
			};
		}
		const subject: MicroWidgetPolicySubject = {
			source: descriptor.source,
			packageId: descriptor.packageId,
			widgetId: descriptor.widgetId,
			policy: descriptor.policy,
		};
		const evaluation = this.evaluate(candidate);
		if (evaluation.status === "granted") {
			const mint = this.mints.get(this.mintKey(candidate));
			if (mint?.kind === "unavailable") {
				return {
					state: this.baselineFrame(descriptor, "unavailable"),
					consent: "granted",
					...idle,
				};
			}
			if (mint?.kind === "failed") {
				return {
					state: {
						status: "error",
						reason: "mint_failed",
						detail: mint.detail,
					},
					consent: "granted",
					...idle,
				};
			}
			return {
				state: this.displayed
					? this.displayedFrame(this.displayed)
					: { status: "minting", subject },
				consent: "granted",
				...idle,
			};
		}
		const current = this.promptCandidate();
		if (this.displayed && evaluation.declaredCovered) {
			return {
				state: this.displayedFrame(this.displayed),
				consent: evaluation.status,
				prompt:
					current?.mode === "runtime"
						? this.buildPrompt(current.candidate, "runtime")
						: null,
				runtimeRequest: this.reviewing
					? null
					: this.computeRuntimeRequest(evaluation),
			};
		}
		if (evaluation.status === "blocked") {
			return {
				state: this.baselineRequested
					? this.baselineFrame(descriptor, "baseline")
					: { status: "blocked", subject, canRunBaseline: true },
				consent: "blocked",
				...idle,
			};
		}
		const prompt = this.buildPrompt(current?.candidate ?? candidate, "mount");
		return {
			state: { status: "pending", subject: prompt.subject },
			consent: evaluation.status,
			prompt,
			runtimeRequest: null,
		};
	}

	private computeNotices(
		state: MicroWidgetGrantState,
	): MicroWidgetGrantNotice[] {
		const notices: MicroWidgetGrantNotice[] = [];
		const frame = state.status === "ready" ? state.frame : null;
		if (frame?.notice === "invalid") {
			notices.push({ kind: "invalid", reason: frame.invalidReason });
		} else if (
			frame?.notice === "baseline" ||
			frame?.notice === "unavailable"
		) {
			notices.push({ kind: frame.notice });
		}
		if (this.runtimeUnavailable) notices.push({ kind: "runtime-unavailable" });
		if (
			this.issues.length > 0 ||
			this.rejections.size > 0 ||
			this.runtimeInvalid !== null
		) {
			notices.push({
				kind: "runtime-skipped",
				rejected: [...this.rejections.values()],
				issues: this.issues,
				invalidReason: this.runtimeInvalid,
			});
		}
		return notices.filter((notice) => !this.dismissedNotices.has(notice.kind));
	}

	private emit(): void {
		const inputs = this.inputs;
		let computed: Pick<
			MicroWidgetGrant,
			"state" | "consent" | "prompt" | "runtimeRequest"
		> = {
			state: { status: "describing" },
			consent: "pending",
			prompt: null,
			runtimeRequest: null,
		};
		if (inputs?.active && this.active && this.d0) {
			computed =
				this.d0.kind === "fallback"
					? {
							...this.computeLegacy(inputs, this.d0.detail),
							runtimeRequest: null,
						}
					: this.computeDescribed(this.d0.descriptor);
		}
		this.snapshot = {
			...computed,
			notices: this.computeNotices(computed.state),
			...this.actions,
		};
		for (const listener of this.listeners) listener();
	}
}

/** Serialized props the widget receives; extraction runs on a JSON copy of exactly that. */
function propsKeyOf(props: Record<string, unknown> | null | undefined): string {
	try {
		return JSON.stringify(publicWidgetProps(props ?? {})) ?? "{}";
	} catch {
		return "{}";
	}
}

/**
 * Mount flow of a package widget, see `MicroWidgetGrantController`. Nothing
 * here reads `component.contract` except the fallback for backends that
 * cannot describe.
 */
export function useMicroWidgetGrant({
	packageId,
	packageVersion,
	bundleHash,
	widgetId,
	preview,
	appId,
	contract,
	props,
	enabled,
}: MicroWidgetGrantInput): MicroWidgetGrant {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const registry =
		(backend as { registryState?: IRegistryState }).registryState ?? null;
	const active = enabled && backendReady;

	const controllerRef = useRef<MicroWidgetGrantController | null>(null);
	controllerRef.current ??= new MicroWidgetGrantController();
	const controller = controllerRef.current;

	// a2ui resolve() hands out fresh contract and props objects on every render.
	const requestedKey = readRequestedCapabilities(contract, preview).join(",");
	const legacyPolicy = useMemo(
		() =>
			capabilityPolicy(
				requestedKey
					? (requestedKey.split(",") as MicroWidgetCapability[])
					: [],
			),
		[requestedKey],
	);
	const legacyAllowed = allowsLegacyMicroWidgetFrame(contract);
	const propsKey = propsKeyOf(props);
	const propsValue = useMemo(
		() => JSON.parse(propsKey) as Record<string, unknown>,
		[propsKey],
	);

	useEffect(() => {
		controller.setInputs({
			registry,
			active,
			packageId,
			packageVersion,
			bundleHash: bundleHash ?? null,
			widgetId,
			preview,
			appId: appId ?? null,
			legacyAllowed,
			legacyPolicy,
			props: propsValue,
			propsKey,
		});
	}, [
		controller,
		registry,
		active,
		packageId,
		packageVersion,
		bundleHash,
		widgetId,
		preview,
		appId,
		legacyAllowed,
		legacyPolicy,
		propsValue,
		propsKey,
	]);

	useEffect(() => {
		controller.activate();
		return () => controller.deactivate();
	}, [controller]);

	return useSyncExternalStore(
		controller.subscribe,
		controller.getSnapshot,
		controller.getSnapshot,
	);
}
