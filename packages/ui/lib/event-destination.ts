import { eventKind } from "./device-management/deployment";
import {
	type EventSinkDefinition,
	isServerOnlyEventType,
	sinkSupportsEventExecution,
} from "./event-definitions";
import { isDeviceEventSource } from "./event-source";

export type Destination = "computer" | "hub" | "device";

/** Fallback order: the first destination that fits wins when the preferred one does not. */
export const DESTINATION_ORDER: readonly Destination[] = [
	"device",
	"computer",
	"hub",
];

export type DestinationReason =
	| "device_unavailable"
	| "device_unsupported"
	| "offline"
	| "board_local"
	| "board_remote"
	| "no_local"
	| "sink_local_only"
	| "sink_remote_only"
	| "hub_unsupported";

export type DestinationHint = "offline_unknown";

export type DestinationStatus =
	| { available: true; hint?: DestinationHint }
	| { available: false; reason: DestinationReason };

export interface DestinationTarget {
	eventType?: string;
	pageId?: string | null;
	sink?: EventSinkDefinition | null;
}

export interface DestinationEnvironment {
	/** The host can create device-only events. */
	deviceCreation: boolean;
	canExecuteLocally: boolean;
	/** `true` blocks the hub; `undefined` while the check loads or after it failed never does. */
	isOffline?: boolean | null;
	offlineCheckFailed?: boolean;
	boardMode?: string | null;
	/** Absent when the hub is not known; then a server-only type is not blocked. */
	hubSupportedSinks?: Readonly<Record<string, boolean | undefined>> | null;
}

const LOCAL = "Local";
const REMOTE = "Remote";

function computerStatus(
	target: DestinationTarget,
	env: DestinationEnvironment,
): DestinationStatus {
	if (
		isServerOnlyEventType(target.eventType) ||
		target.sink?.availability === "remote"
	)
		return { available: false, reason: "sink_remote_only" };
	if (!env.canExecuteLocally) return { available: false, reason: "no_local" };
	if (env.boardMode === REMOTE)
		return { available: false, reason: "board_remote" };
	return { available: true };
}

function hubStatus(
	target: DestinationTarget,
	env: DestinationEnvironment,
): DestinationStatus {
	if (!sinkSupportsEventExecution(target.sink, REMOTE, env.canExecuteLocally))
		return { available: false, reason: "sink_local_only" };
	if (
		isServerOnlyEventType(target.eventType) &&
		env.hubSupportedSinks &&
		env.hubSupportedSinks[target.eventType ?? ""] !== true
	)
		return { available: false, reason: "hub_unsupported" };
	if (env.isOffline === true) return { available: false, reason: "offline" };
	if (env.boardMode === LOCAL)
		return { available: false, reason: "board_local" };
	return env.offlineCheckFailed
		? { available: true, hint: "offline_unknown" }
		: { available: true };
}

function deviceStatus(
	target: DestinationTarget,
	env: DestinationEnvironment,
): DestinationStatus {
	if (!env.deviceCreation)
		return { available: false, reason: "device_unavailable" };
	if (
		target.eventType &&
		eventKind({
			event_type: target.eventType,
			default_page_id: target.pageId,
		}) === null
	)
		return { available: false, reason: "device_unsupported" };
	return { available: true };
}

export function destinationStatus(
	destination: Destination,
	target: DestinationTarget,
	env: DestinationEnvironment,
): DestinationStatus {
	if (destination === "device") return deviceStatus(target, env);
	if (destination === "computer") return computerStatus(target, env);
	return hubStatus(target, env);
}

export type DestinationStatuses = Record<Destination, DestinationStatus>;

export function destinationStatuses(
	target: DestinationTarget,
	env: DestinationEnvironment,
): DestinationStatuses {
	return {
		device: deviceStatus(target, env),
		computer: computerStatus(target, env),
		hub: hubStatus(target, env),
	};
}

export interface DestinationResolution {
	destination: Destination;
	/** No destination fits; `destination` is the preferred one. */
	none: boolean;
	/** Set when the preferred destination did not fit and another was taken. */
	fellBackFrom?: { destination: Destination; reason: DestinationReason };
}

export function resolveDestination(
	preferred: Destination,
	statuses: DestinationStatuses,
): DestinationResolution {
	const wanted = statuses[preferred];
	if (wanted.available) return { destination: preferred, none: false };
	const next = DESTINATION_ORDER.find(
		(candidate) => statuses[candidate].available,
	);
	if (!next) return { destination: preferred, none: true };
	return {
		destination: next,
		none: false,
		fellBackFrom: { destination: preferred, reason: wanted.reason },
	};
}

/** The destination a new form opens on, from the host and from a template event, if there is one. */
export function initialDestination(
	env: Pick<DestinationEnvironment, "deviceCreation" | "canExecuteLocally">,
	template?: {
		config?: number[] | null;
		execution_mode?: string | null;
	} | null,
): Destination {
	const standard: Destination = env.deviceCreation
		? "device"
		: env.canExecuteLocally
			? "computer"
			: "hub";
	if (!template) return standard;
	if (
		env.deviceCreation &&
		isDeviceEventSource({ config: template.config ?? [] })
	)
		return "device";
	if (template.execution_mode === REMOTE) return "hub";
	if (template.execution_mode === LOCAL && env.canExecuteLocally)
		return "computer";
	return standard;
}

const REASON_PRIORITY: readonly DestinationReason[] = [
	"hub_unsupported",
	"offline",
	"board_local",
	"sink_local_only",
	"no_local",
	"board_remote",
	"sink_remote_only",
	"device_unsupported",
	"device_unavailable",
];

/** Why a trigger type fits no destination at all; null when at least one fits. */
export function blockingReason(
	statuses: DestinationStatuses,
): DestinationReason | null {
	const reasons: DestinationReason[] = [];
	for (const destination of DESTINATION_ORDER) {
		const status = statuses[destination];
		if (status.available) return null;
		reasons.push(status.reason);
	}
	return REASON_PRIORITY.find((reason) => reasons.includes(reason)) ?? null;
}

/** The execution target a schedule carries; a device carries none. */
export function destinationSinkExecution(
	destination: Destination,
): "LOCAL" | "REMOTE" | undefined {
	if (destination === "computer") return "LOCAL";
	if (destination === "hub") return "REMOTE";
	return undefined;
}

type Translate = (
	key: string,
	defaultValue: string,
	options?: { type: string },
) => string;

export function destinationReasonText(
	t: Translate,
	reason: DestinationReason,
	eventType?: string,
): string {
	switch (reason) {
		case "device_unavailable":
			return t(
				"common:destinationDeviceUnavailable",
				"Devices can't be set up from here.",
			);
		case "device_unsupported":
			return t(
				"common:destinationDeviceUnsupported",
				"This trigger can't run on devices.",
			);
		case "offline":
			return t(
				"common:destinationOffline",
				"This app is offline. Sync it to an online profile first.",
			);
		case "board_local":
			return t("common:destinationBoardLocal", "This flow only runs locally.");
		case "board_remote":
			return t(
				"common:destinationBoardRemote",
				"This flow only runs on the server.",
			);
		case "no_local":
			return t("common:destinationNoLocal", "Needs the desktop app.");
		case "sink_local_only":
			return t(
				"common:destinationSinkLocalOnly",
				"This trigger only runs in the desktop app.",
			);
		case "sink_remote_only":
			return t(
				"common:destinationSinkRemoteOnly",
				"This trigger runs on the hub.",
			);
		case "hub_unsupported":
			return t(
				"common:destinationHubUnsupported",
				"This hub doesn't support {{type}}.",
				{ type: eventType ?? "" },
			);
	}
}

export function destinationHintText(
	t: Translate,
	hint: DestinationHint,
): string {
	return hint === "offline_unknown"
		? t(
				"common:destinationOfflineUnknown",
				"Couldn't check the connection. It is checked again when you create.",
			)
		: "";
}
