import type {
	AgeState,
	CopyRef,
	Freshness,
	FreshnessReason,
	FreshnessSignal,
	SourcePlane,
} from "./types";
import { SOURCE_PLANES } from "./types";

export interface FreshnessRule {
	src: SourcePlane;
	/** Age ≤ currentS is current; on the live plane it is live while the session is open. */
	currentS?: number;
	/** Age ≤ delayedS is delayed; anything older is last known. */
	delayedS?: number;
	/** Producer or app cadence, for the stamp tooltip. */
	cadenceS?: number;
	/** Saved inventory never claims current health. */
	fixed?: "snapshot";
}

/** IA §2.1, one row per signal. */
export const FRESHNESS_RULES: Readonly<Record<FreshnessSignal, FreshnessRule>> =
	{
		device_row: { src: "hub", cadenceS: 30 },
		presence: { src: "hub", currentS: 120, delayedS: 600, cadenceS: 60 },
		cert_inventory: { src: "hub", currentS: 7_200, cadenceS: 3_600 },
		hub_support: { src: "hub" },
		readiness: { src: "hub" },
		policy: { src: "hub" },
		resources: { src: "hub", cadenceS: 30 },
		archive_list: { src: "hub", cadenceS: 30 },
		fleet_status: { src: "snap", currentS: 90, delayedS: 600, cadenceS: 60 },
		fleet_metrics: { src: "snap", currentS: 75, cadenceS: 30 },
		saved_inventory: { src: "saved", fixed: "snapshot" },
		live_inspection: { src: "live", currentS: 30, cadenceS: 20 },
		live_metrics: { src: "live", currentS: 15, cadenceS: 5 },
		project_metrics: { src: "live", currentS: 15, cadenceS: 5 },
		logs: { src: "live", currentS: 15, cadenceS: 5 },
		certificates_live: { src: "live", currentS: 120, cadenceS: 60 },
		offline_queues: { src: "live", currentS: 30, cadenceS: 15 },
		rollout: { src: "live", currentS: 10, cadenceS: 1 },
		host_operation: { src: "live", currentS: 30, cadenceS: 10 },
		session: { src: "live" },
		local_vault: { src: "local" },
		agent_local: { src: "device" },
	};

/** |skew| above this is flagged on the stamp (IA §2.2 "Clock safety"). */
export const CLOCK_SKEW_FLAG_S = 120;

export interface ClassifyInput {
	/** Hub-corrected unix seconds. */
	now: number;
	/** When the shown data was produced or read; `null` = loaded but never reported. */
	at?: number | null;
	loaded: boolean;
	sessionOpen?: boolean;
	locked?: boolean;
	error?: Freshness["error"];
	/** With `error`: when the shown data was read (defaults to `at`). */
	dataFrom?: number;
	noAccess?: CopyRef<FreshnessReason>;
	unsupported?: CopyRef<FreshnessReason>;
	notLoadedReason?: CopyRef<FreshnessReason>;
	/** Known skew of the producer's clock against the hub, seconds. */
	skewS?: number;
	/** R5: a failing hub turns every Hub stamp into "couldn't refresh · data from …". */
	hubFailing?: { error: NonNullable<Freshness["error"]>; dataFrom?: number };
	/** R5 exception: per-row "device reported at" stamps keep their age. */
	noFail?: boolean;
}

export function classify(
	signal: FreshnessSignal,
	input: ClassifyInput,
): Freshness {
	const rule = FRESHNESS_RULES[signal];
	const at = input.at ?? undefined;
	const base: Freshness = { src: rule.src, age: "notloaded" };
	if (rule.cadenceS !== undefined) base.cadenceS = rule.cadenceS;
	if (at !== undefined) base.at = at;

	if (input.unsupported)
		return { ...base, age: "unsupported", reason: input.unsupported };
	if (input.noAccess)
		return { ...base, age: "noaccess", reason: input.noAccess };
	if (input.locked)
		return { ...base, age: "locked", reason: { code: "unlock_required" } };

	const hubError =
		rule.src === "hub" && !input.noFail ? input.hubFailing : undefined;
	const error = input.error ?? hubError?.error;
	if (error) {
		const failed: Freshness = { ...base, age: "error", error };
		const dataFrom = input.error
			? (input.dataFrom ?? at)
			: (hubError?.dataFrom ?? input.dataFrom ?? at);
		if (input.loaded && dataFrom !== undefined) failed.dataFrom = dataFrom;
		return withSkew(failed, input, at);
	}

	if (!input.loaded)
		return input.notLoadedReason
			? { ...base, reason: input.notLoadedReason }
			: base;
	if (input.at === null) return { ...base, reason: { code: "never_reported" } };

	return withSkew({ ...base, age: ageOf(rule, input, at) }, input, at);
}

const ageOf = (
	rule: FreshnessRule,
	input: ClassifyInput,
	at: number | undefined,
): AgeState => {
	if (rule.fixed) return rule.fixed;
	const ageS = at === undefined ? undefined : Math.max(0, input.now - at);
	if (rule.src === "live") return liveAge(rule, input.sessionOpen, ageS);
	if (ageS === undefined)
		return rule.src === "hub" || rule.src === "local" ? "current" : "lastknown";
	return thresholdAge(rule, ageS);
};

const liveAge = (
	rule: FreshnessRule,
	sessionOpen: boolean | undefined,
	ageS: number | undefined,
): AgeState => {
	const fresh =
		rule.currentS === undefined ||
		(ageS !== undefined && ageS <= rule.currentS);
	return sessionOpen && fresh ? "live" : "lastknown";
};

const thresholdAge = (rule: FreshnessRule, ageS: number): AgeState => {
	if (rule.currentS === undefined || ageS <= rule.currentS) return "current";
	if (rule.delayedS !== undefined && ageS <= rule.delayedS) return "delayed";
	return "lastknown";
};

const flaggedSkew = (skewS: number | undefined) =>
	skewS !== undefined && Math.abs(skewS) > CLOCK_SKEW_FLAG_S
		? skewS
		: undefined;

const withSkew = (
	freshness: Freshness,
	input: ClassifyInput,
	at: number | undefined,
): Freshness => {
	const negativeAge =
		at !== undefined && at > input.now ? input.now - at : undefined;
	const skewS = flaggedSkew(input.skewS) ?? negativeAge;
	return skewS === undefined ? freshness : { ...freshness, skewS };
};

function sourceKey(freshness: Freshness): string {
	const code = freshness.error?.code ?? freshness.reason?.code ?? "";
	return `${freshness.src}|${freshness.age}|${code}`;
}

/** R5: the block head stamp is the source + age shared by at least two rows (Hub wins ties), dated by its oldest row. */
export function baseSource(
	rows: readonly (Freshness | undefined)[],
): Freshness | undefined {
	const groups = new Map<string, Freshness[]>();
	for (const row of rows) {
		if (!row) continue;
		const key = sourceKey(row);
		groups.set(key, [...(groups.get(key) ?? []), row]);
	}
	let best: Freshness[] = [];
	for (const group of groups.values())
		if (
			group.length > best.length ||
			(group.length === best.length &&
				group[0].src === "hub" &&
				best[0]?.src !== "hub")
		)
			best = group;
	if (best.length < 2) return undefined;
	return best.reduce((oldest, row) =>
		row.at !== undefined && (oldest.at === undefined || row.at < oldest.at)
			? row
			: oldest,
	);
}

/** R5: a row repeats its stamp only when its source or age state differs from the block's. */
export function sameSource(
	row: Freshness | undefined,
	base: Freshness | undefined,
): boolean {
	return !!row && !!base && sourceKey(row) === sourceKey(base);
}

/** Higher is worse; the status bar at ≤ 720 px shows the worst plane. */
export const AGE_RANK: Readonly<Record<AgeState, number>> = {
	live: 0,
	current: 0,
	snapshot: 1,
	notloaded: 2,
	unsupported: 3,
	noaccess: 4,
	locked: 5,
	delayed: 6,
	lastknown: 7,
	error: 8,
};

export function worstPlane(
	planes: Partial<Record<SourcePlane, Freshness>>,
): SourcePlane | undefined {
	let worst: SourcePlane | undefined;
	for (const plane of SOURCE_PLANES) {
		const freshness = planes[plane];
		if (!freshness) continue;
		const current = worst ? planes[worst] : undefined;
		if (!current || AGE_RANK[freshness.age] > AGE_RANK[current.age])
			worst = plane;
	}
	return worst;
}
