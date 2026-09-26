import type {
	MediaPlayPayload,
	MediaResultPayload,
	WidgetMediaState,
} from "./media";
import type {
	AudioRequestPayload,
	MicrophoneResultPayload,
	RequestIdPayload,
} from "./microphone";

export const FLW_PROTOCOL = "flw/1";

export type ThemeMode = "light" | "dark";

export interface ThemeState {
	mode: ThemeMode;
	tokens: Record<string, string>;
}

export interface InitCapabilities {
	preview?: boolean;
	microphone?: boolean;
	media?: boolean;
	mediaIds?: string[];
}

export interface InitPayload {
	props: Record<string, unknown>;
	theme: ThemeState;
	locale: string;
	instanceId: string;
	capabilities: InitCapabilities;
}

export interface PropsUpdatePayload {
	props: Record<string, unknown>;
}

export interface QueryPayload {
	queryId: string;
	name: string;
	args: unknown;
}

export type HelloPayload = Record<string, never>;

export interface ReadyPayload {
	contractVersion: number;
}

export interface EventPayload {
	name: string;
	payload: unknown;
}

export interface QueryResultPayload {
	queryId: string;
	ok: boolean;
	value?: unknown;
	error?: string;
}

export interface ResizePayload {
	height: number;
}

export interface ValueChangedPayload {
	values: Record<string, unknown>;
}

export interface FlwPayloadMap {
	"media:play": MediaPlayPayload;
	"media:pause": Record<string, never>;
	"media:stop": Record<string, never>;
	"media:result": MediaResultPayload;
	"media:state": WidgetMediaState;
	"microphone:request": AudioRequestPayload;
	"microphone:result": MicrophoneResultPayload;
	"microphone:stop": RequestIdPayload;
	"microphone:cancel": RequestIdPayload;
	"capabilities:update": InitCapabilities;
	init: InitPayload;
	"props:update": PropsUpdatePayload;
	"theme:change": ThemeState;
	query: QueryPayload;
	hello: HelloPayload;
	ready: ReadyPayload;
	event: EventPayload;
	"query:result": QueryResultPayload;
	resize: ResizePayload;
	"value:changed": ValueChangedPayload;
}

export type HostToWidgetMessageType =
	| "media:result"
	| "media:state"
	| "microphone:result"
	| "capabilities:update"
	| "init"
	| "props:update"
	| "theme:change"
	| "query";

export type WidgetToHostMessageType =
	| "media:play"
	| "media:pause"
	| "media:stop"
	| "microphone:request"
	| "microphone:stop"
	| "microphone:cancel"
	| "hello"
	| "ready"
	| "event"
	| "query:result"
	| "resize"
	| "value:changed";

export type FlwMessageType = HostToWidgetMessageType | WidgetToHostMessageType;

export interface FlwEnvelope<T extends FlwMessageType = FlwMessageType> {
	protocol: typeof FLW_PROTOCOL;
	nonce: string;
	instanceId: string;
	type: T;
	payload: T extends keyof FlwPayloadMap ? FlwPayloadMap[T] : unknown;
}

const FLW_MESSAGE_TYPES: ReadonlySet<string> = new Set([
	"media:play",
	"media:pause",
	"media:stop",
	"media:result",
	"media:state",
	"microphone:request",
	"microphone:result",
	"microphone:stop",
	"microphone:cancel",
	"capabilities:update",
	"init",
	"props:update",
	"theme:change",
	"query",
	"hello",
	"ready",
	"event",
	"query:result",
	"resize",
	"value:changed",
]);

export function isFlwEnvelope(value: unknown): value is FlwEnvelope {
	if (typeof value !== "object" || value === null) return false;
	const candidate = value as Record<string, unknown>;
	return (
		candidate.protocol === FLW_PROTOCOL &&
		typeof candidate.nonce === "string" &&
		typeof candidate.instanceId === "string" &&
		typeof candidate.type === "string" &&
		FLW_MESSAGE_TYPES.has(candidate.type) &&
		"payload" in candidate
	);
}

export function createEnvelope<T extends FlwMessageType>(
	type: T,
	payload: FlwPayloadMap[T],
	nonce: string,
	instanceId: string,
): FlwEnvelope<T> {
	return {
		protocol: FLW_PROTOCOL,
		nonce,
		instanceId,
		type,
		payload,
	} as FlwEnvelope<T>;
}
