/**
 * FlowPilot diagnostics may contain detailed prompts, tool arguments and results.
 * Keep both their runtime overhead and their UI out of production builds.
 */
export function isFlowPilotDebugEnabled(
	environment = process.env.NODE_ENV,
): boolean {
	return environment === "development";
}

export const FLOWPILOT_DEBUG_ENABLED = isFlowPilotDebugEnabled();

/** Console logging for FlowPilot internals. Operational errors stay on console.error. */
export function flowPilotDebugLog(...args: unknown[]) {
	if (FLOWPILOT_DEBUG_ENABLED) console.debug(...args);
}

/** Remove a persisted debug report without mutating the caller's message object. */
export function stripFlowPilotDebugReport<T extends { debug_report?: unknown }>(
	value: T,
	debugEnabled = FLOWPILOT_DEBUG_ENABLED,
): T {
	if (debugEnabled || value.debug_report === undefined) return value;
	const sanitized = { ...value };
	delete sanitized.debug_report;
	return sanitized;
}
