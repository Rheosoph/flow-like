import { EVENT_DEFINITIONS } from "../event-definitions";

/** Generated identifiers belong to each new Event; the remaining defaults are shared. */
export function buildNativeEventDefaults() {
	const defaults: Record<string, Record<string, unknown>> = {};
	for (const definition of Object.values(EVENT_DEFINITIONS)) {
		for (const [type, config] of Object.entries(definition.configs)) {
			const value = { ...config } as Record<string, unknown>;
			if (type === "api") delete value.path;
			if (type === "deeplink") delete value.route;
			defaults[type] = value;
		}
	}
	return defaults;
}
