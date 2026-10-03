import type { IEvent } from "./schema/flow/event";

/** Stored beside trigger settings so every event transport retains the source preference. */
const SOURCE_KEY = "__flow_like_source";

function configObject(
	config: readonly number[] | undefined,
): Record<string, unknown> | null {
	if (!config?.length) return {};
	try {
		const value: unknown = JSON.parse(
			new TextDecoder().decode(new Uint8Array(config)),
		);
		return value && typeof value === "object" && !Array.isArray(value)
			? (value as Record<string, unknown>)
			: null;
	} catch {
		return null;
	}
}

const encode = (value: Record<string, unknown>): number[] =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

export function isDeviceEventSource(event: Pick<IEvent, "config">): boolean {
	return configObject(event.config)?.[SOURCE_KEY] === "device";
}

export function withDeviceEventSource<T extends Pick<IEvent, "config">>(
	event: T,
): T {
	const config = configObject(event.config);
	if (!config) throw new Error("Device event settings must be a JSON object.");
	return { ...event, config: encode({ ...config, [SOURCE_KEY]: "device" }) };
}

/** Trigger editors receive only their settings, without deployment metadata. */
export function eventTriggerConfig(config: number[]): number[] {
	const value = configObject(config);
	if (!value || !Object.hasOwn(value, SOURCE_KEY)) return config;
	delete value[SOURCE_KEY];
	return encode(value);
}

/** A trigger editor may replace all settings; the event's source preference survives. */
export function mergeEventTriggerConfig(
	previous: number[],
	next: number[],
): number[] {
	return isDeviceEventSource({ config: previous })
		? withDeviceEventSource({ config: next }).config
		: eventTriggerConfig(next);
}
