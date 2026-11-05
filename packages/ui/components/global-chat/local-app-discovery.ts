/**
 * Whether a `list_apps` result opens FlowPilot's research and HTTP fallbacks.
 *
 * The question is only whether a local inventory came back, never whether every app in it read
 * cleanly. `list_apps` reports `partial` when an app's Events cannot be loaded, when a current
 * profile app is missing from the backend inventory, or when the listing hits its safety cap, and
 * each of those is still evidence about what exists locally. Demanding a clean `ok` sealed the
 * fallback off for the rest of the run, so `research_agent` failed on every attempt no matter how
 * many times discovery succeeded.
 *
 * A failed, timed-out, or absent result is not an inventory and keeps the gate shut: app absence
 * stays unproven until a listing actually returns.
 *
 * Mirrors `returned_app_inventory_result` in
 * packages/core/editor/src/flow/copilot/platform.rs. Both hosts must open the fallback on the same
 * results, or the same run behaves differently depending on which backend serves it.
 */
export function isReturnedAppInventory(result: unknown): boolean {
	if (!result || typeof result !== "object") return false;
	const status = (result as Record<string, unknown>).status;
	return status === "ok" || status === "partial";
}

/** Why the sealed public researcher may not run, or `undefined` when it may. */
export interface SealedResearchRefusal {
	code: string;
	message: string;
	retryable?: boolean;
}

/** The run-scoped facts the gate decides on. */
export interface SealedResearchRoutingState {
	appInventoryReturned: boolean;
}

const FRESH_RUN: SealedResearchRoutingState = {
	appInventoryReturned: false,
};

function localAppDiscoveryRefusal(
	state: SealedResearchRoutingState | undefined,
): SealedResearchRefusal | undefined {
	if (!state?.appInventoryReturned)
		return {
			code: "local_app_discovery_required",
			retryable: true,
			message:
				"Call list_apps and wait for its result first. Use an existing app if a configured interface handles the request; otherwise use web research or a direct API request. A partial inventory counts as discovery, but a failed listing does not.",
		};
	return undefined;
}

/** Direct HTTP calls share the app discovery gate and have no one-shot limit. */
export function httpRequestRefusal(
	runId: string | undefined,
	state: SealedResearchRoutingState | undefined,
): SealedResearchRefusal | undefined {
	return localAppDiscoveryRefusal(runId?.trim() ? state : undefined);
}

/**
 * A returned app inventory must precede research. The run id keeps follow-up calls attached
 * to the same research budget and source ledger.
 */
export function sealedResearchRefusal(
	runId: string | undefined,
	state: SealedResearchRoutingState | undefined,
): SealedResearchRefusal | undefined {
	if (!runId?.trim())
		return {
			code: "sealed_research_unavailable",
			message:
				"This run has no id, so its research budget cannot be tracked. Retry in a new turn.",
		};
	const current = state ?? FRESH_RUN;
	return localAppDiscoveryRefusal(current);
}
