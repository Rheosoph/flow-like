import type {
	HubError,
	HubStandalone,
} from "../../../../lib/device-management/hub/endpoints";
import {
	type ReleaseCheck,
	type ReleaseFacts,
	ReleaseVerificationError,
	type VerifiedRelease,
} from "../../../../lib/device-package";

export const DAY = 86_400;
/** From here on the end of the release is a warning instead of a date. */
export const RELEASE_ENDS_SOON_S = 30 * DAY;

/** The release list arrived and failed a check, as opposed to not arriving. */
export interface ReleaseCheckFailure {
	check: ReleaseCheck;
	/** What the list says about itself; only when its signature verified. */
	facts?: ReleaseFacts;
	/** The verifier's own English sentence, for a hover or a report. */
	detail: string;
}

/** The verifier's error travels as the cause of the `HubError` the read fails with. */
export function releaseCheckFailure(
	error: HubError | undefined,
): ReleaseCheckFailure | undefined {
	const cause = error?.cause;
	if (!(cause instanceof ReleaseVerificationError)) return undefined;
	return {
		check: cause.check,
		...(cause.facts ? { facts: cause.facts } : {}),
		detail: cause.message,
	};
}

const factsOf = ({ manifest }: VerifiedRelease): ReleaseFacts => ({
	release_version: manifest.release_version,
	sequence: manifest.sequence,
	issued_at: manifest.issued_at,
	expires_at: manifest.expires_at,
});

/**
 * What this computer knows about the hub's agent release, as one answer for
 * every place that shows it. Positive only with a verification result in hand,
 * and never while the newest attempt failed a check.
 */
export type ReleaseVerdict =
	/** The hub's release settings haven't been read yet. */
	| { kind: "waiting" }
	| { kind: "missing" }
	| { kind: "checking" }
	/** The list didn't arrive and nothing was verified before. */
	| { kind: "unfetched"; error: HubError }
	| {
			kind: "failed";
			check: Exclude<ReleaseCheck, "expired">;
			facts?: ReleaseFacts;
			detail: string;
	  }
	/** Verified earlier and past its end now, or refused as run out. `release` only for the first. */
	| { kind: "expired"; facts: ReleaseFacts; release?: VerifiedRelease }
	| {
			kind: "ok" | "ends_soon";
			release: VerifiedRelease;
			facts: ReleaseFacts;
			/** Whole days until the end, rounded up. */
			daysLeft: number;
			/** The newest attempt didn't reach the list: the verdict is from the last verification. */
			unreached?: HubError;
	  };

export type ReleaseVerdictKind = ReleaseVerdict["kind"];

/** The newest read of the release list: what verified last, and how the newest attempt failed. */
export interface ReleaseAttempt {
	data?: VerifiedRelease;
	error?: HubError;
}

/** Whole days left of a validity window, never negative. */
export const daysLeft = (expiresAt: number, nowS: number) =>
	Math.max(0, Math.ceil((expiresAt - nowS) / DAY));

export function releaseVerdictOf(
	record: HubStandalone | undefined,
	release: ReleaseAttempt,
	nowS: number,
): ReleaseVerdict {
	if (!record) return { kind: "waiting" };
	if (!record.release_trust) return { kind: "missing" };
	const failure = releaseCheckFailure(release.error);
	if (failure) {
		const { check, facts, detail } = failure;
		if (check === "expired" && facts) return { kind: "expired", facts };
		return {
			kind: "failed",
			check: check === "expired" ? "invalid" : check,
			...(facts ? { facts } : {}),
			detail,
		};
	}
	const verified = release.data;
	if (!verified)
		return release.error
			? { kind: "unfetched", error: release.error }
			: { kind: "checking" };
	const facts = factsOf(verified);
	const leftS = facts.expires_at - nowS;
	if (leftS <= 0) return { kind: "expired", facts, release: verified };
	return {
		kind: leftS <= RELEASE_ENDS_SOON_S ? "ends_soon" : "ok",
		release: verified,
		facts,
		daysLeft: daysLeft(facts.expires_at, nowS),
		...(release.error ? { unreached: release.error } : {}),
	};
}

/** The release in hand, only while the verdict lets it be used. */
export const usableRelease = (
	verdict: ReleaseVerdict,
): VerifiedRelease | undefined =>
	verdict.kind === "ok" || verdict.kind === "ends_soon"
		? verdict.release
		: undefined;
