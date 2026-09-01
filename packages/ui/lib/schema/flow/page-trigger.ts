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
