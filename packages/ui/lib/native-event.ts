import { classifyAppEventInterface } from "../components/global-chat/app-event-interface";
import type { IEvent } from "./schema/flow/event";
import { parseUint8ArrayToJson } from "./uint8";

export const NATIVE_SURFACES = [
	"siri",
	"shortcuts",
	"widget",
	"spotlight",
] as const;
export type NativeSurface = (typeof NATIVE_SURFACES)[number];

/** Stored inside the Event's versioned config bytes, alongside its trigger settings. */
export interface NativeEventSettings {
	enabled: boolean;
	surfaces: NativeSurface[];
	favorite: boolean;
	operation?: string;
}

export function nativeEventSettings(
	event: Pick<IEvent, "config">,
): NativeEventSettings {
	let value: unknown;
	try {
		value = parseUint8ArrayToJson(event.config ?? [])?.native_integration;
	} catch {
		value = undefined;
	}
	const config =
		value && typeof value === "object"
			? (value as Record<string, unknown>)
			: {};
	return {
		enabled: config.enabled === true,
		favorite: config.favorite === true,
		operation:
			typeof config.operation === "string"
				? config.operation.trim() || undefined
				: undefined,
		surfaces: NATIVE_SURFACES.filter(
			(surface) =>
				Array.isArray(config.surfaces) && config.surfaces.includes(surface),
		),
	};
}

export function nativeEventActionKind(
	event: IEvent,
): "open_event" | "run_event" | undefined {
	if (event.event_type === "mcp")
		return event.execution_mode === "Remote" &&
			nativeEventSettings(event).operation
			? "run_event"
			: undefined;
	const kind = classifyAppEventInterface(event);
	if (kind === "page" || kind === "chat") return "open_event";
	if (["quick_action", "generic_form", "api"].includes(event.event_type))
		return "run_event";
	return undefined;
}

export function isNativeEventExposed(event: IEvent): boolean {
	const settings = nativeEventSettings(event);
	return (
		event.active &&
		settings.enabled &&
		settings.surfaces.length > 0 &&
		nativeEventActionKind(event) !== undefined
	);
}
