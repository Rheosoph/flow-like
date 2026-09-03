/**
 * Opaque routing reference attached to a Page action by the trusted Page
 * projection or by a workflow that created the action at runtime.
 *
 * `capabilityJwt` proves a dynamic binding. It is request data and must never
 * replace the caller's normal Authorization header.
 */
export interface PageActionInvocation {
	actionId: string;
	capabilityJwt?: string;
	manifestRevision?: string;
}

export type PageSpecialEvent = "load" | "unload" | "interval";

/** A Page Event selector understood by prerun and invocation endpoints. */
export type PageTrigger =
	| ({ kind: "action" } & PageActionInvocation)
	| {
			kind: "special";
			specialEvent: PageSpecialEvent;
			manifestRevision: string;
	  };

/** Snake-case request shape consumed by the Rust API. */
export type PageTriggerRequest =
	| {
			kind: "action";
			action_id: string;
			capability_jwt?: string;
			manifest_revision?: string;
	  }
	| {
			kind: "special";
			special_event: PageSpecialEvent;
			manifest_revision: string;
	  };

export function pageTriggerFromAction(
	invocation: PageActionInvocation,
): PageTrigger {
	return { kind: "action", ...invocation };
}

/**
 * A hosted Page must dispatch the opaque route attached by its trusted Page
 * projection. The same rule applies to native Pages because dynamic output
 * without a signed Page action must not escape through direct Board execution.
 */
export function mayDispatchRawPageBoardAction(
	governedPage: boolean | undefined,
): boolean {
	return governedPage !== true;
}

export function serializePageTrigger(trigger: PageTrigger): PageTriggerRequest {
	if (trigger.kind === "special") {
		return {
			kind: "special",
			special_event: trigger.specialEvent,
			manifest_revision: trigger.manifestRevision,
		};
	}
	return {
		kind: "action",
		action_id: trigger.actionId,
		...(trigger.capabilityJwt ? { capability_jwt: trigger.capabilityJwt } : {}),
		...(trigger.manifestRevision
			? { manifest_revision: trigger.manifestRevision }
			: {}),
	};
}

export const PAGE_ACTION_ID_PREFIX = "pa1_";
export const SERVER_DYNAMIC_PAGE_ACTION_ID_PREFIX = "da1_";
export const LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX = "lda1_";

/**
 * A grant minted at run time against one exact manifest revision — either a
 * local registry entry (`lda1_`) or a server capability JWT (`da1_`). Their
 * authority dies with that revision, so they are never re-stamped.
 */
export function isDynamicPageTrigger(trigger?: PageTrigger): boolean {
	if (trigger?.kind !== "action") return false;
	return (
		trigger.actionId.startsWith(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX) ||
		trigger.actionId.startsWith(SERVER_DYNAMIC_PAGE_ACTION_ID_PREFIX) ||
		Boolean(trigger.capabilityJwt)
	);
}

/**
 * Re-stamp a compiled trigger with the revision an authority just reported for
 * this same Event.
 *
 * The manifest revision hashes the whole Board, so a rendered Page carries a
 * revision that any unrelated edit supersedes. The authority that is about to
 * judge the run already told us the current one — send that instead of the one
 * the surface was built with, rather than making the user reload.
 *
 * `current` MUST come from a response the judging authority itself produced for
 * this `(appId, eventId)` — a prerun answer, or `get_local_page_bootstrap`.
 * Never from rendered Page content: `strip_spoofed_page_action_metadata`
 * (packages/core/src/flow/compiled/prerun.rs) exists precisely because a Page
 * payload can carry forged `pageAction` blobs.
 *
 * Returns the SAME reference whenever substitution is not legitimate, so a
 * caller detects real drift with `next !== trigger`.
 */
export function withCurrentManifestRevision<T extends PageTrigger>(
	trigger: T,
	current: string | undefined | null,
): T {
	const fresh = current?.trim();
	// Never blank it: serializePageTrigger drops a falsy revision, and both
	// native and server gates reject an absent one outright.
	if (!fresh) return trigger;
	// Never resurrect a grant that was minted against one exact revision.
	if (isDynamicPageTrigger(trigger)) return trigger;
	// Never mint provenance the surface never had — carrying a revision at all
	// is what proves the caller came through a real bootstrap.
	if (!trigger.manifestRevision?.trim()) return trigger;
	if (trigger.manifestRevision === fresh) return trigger;
	return { ...trigger, manifestRevision: fresh };
}
