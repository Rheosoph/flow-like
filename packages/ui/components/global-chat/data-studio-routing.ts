export const DATA_STUDIO_ROUTING_REASONS = [
	"build",
	"explicit_raw_data",
	"no_suitable_event",
	"event_insufficient",
] as const;

export type DataStudioRoutingReason =
	(typeof DATA_STUDIO_ROUTING_REASONS)[number];

export type DataStudioRoutingGateError = {
	status: "error";
	code:
		| "data_studio_routing_reason_required"
		| "data_studio_routing_reason_invalid"
		| "event_preflight_required";
	message: string;
	retryable: true;
	required_argument?: "routing_reason";
	allowed_values?: readonly DataStudioRoutingReason[];
	routing_reason?: string;
	required_tool?: "list_apps";
	inventory_complete?: false;
	next_action: string;
};

export interface DataStudioRoutingGateInput {
	routingReason: unknown;
	appInventoryComplete: boolean;
}

function isDataStudioRoutingReason(
	value: string,
): value is DataStudioRoutingReason {
	return DATA_STUDIO_ROUTING_REASONS.some((reason) => reason === value);
}

/**
 * Keeps use-time Data Studio access behind a completed app/Event inventory while preserving
 * independent BUILD lanes. The host owns `appInventoryComplete`; the model can explain why it is
 * asking for raw data access, but cannot claim that discovery completed.
 */
export function dataStudioRoutingGate({
	routingReason,
	appInventoryComplete,
}: DataStudioRoutingGateInput): DataStudioRoutingGateError | null {
	const normalized =
		typeof routingReason === "string" ? routingReason.trim() : "";
	if (!normalized) {
		return {
			status: "error",
			code: "data_studio_routing_reason_required",
			message:
				"data_studio_agent requires a routing_reason so configured app Events remain the first choice for use-time work.",
			retryable: true,
			required_argument: "routing_reason",
			allowed_values: DATA_STUDIO_ROUTING_REASONS,
			next_action:
				"Retry data_studio_agent with one allowed routing_reason. Use 'build' only for app-building data work.",
		};
	}
	if (!isDataStudioRoutingReason(normalized)) {
		return {
			status: "error",
			code: "data_studio_routing_reason_invalid",
			message: `Unknown data_studio_agent routing_reason '${normalized}'.`,
			retryable: true,
			required_argument: "routing_reason",
			allowed_values: DATA_STUDIO_ROUTING_REASONS,
			routing_reason: normalized,
			next_action:
				"Retry data_studio_agent with one allowed routing_reason. Use 'build' only for app-building data work.",
		};
	}
	if (normalized === "build" || appInventoryComplete) return null;

	return {
		status: "error",
		code: "event_preflight_required",
		message:
			"A complete prior list_apps result is required before Data Studio can be used as a use-time fallback.",
		retryable: true,
		routing_reason: normalized,
		required_tool: "list_apps",
		inventory_complete: false,
		next_action:
			"Call list_apps, wait for status 'ok' with complete: true, inspect the configured Events, then retry data_studio_agent with the same routing_reason only if Data Studio is still necessary.",
	};
}
