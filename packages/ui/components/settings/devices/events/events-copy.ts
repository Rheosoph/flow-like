import { eligibilityCopy } from "../copy/eligibility-copy";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "../primitives/area-context";
import type { GateKind } from "../primitives/icons";
import { convergenceLabel } from "../primitives/status-chip";
import type { EventsDevicesBlock, EventsDevicesValue } from "./events-devices";
import type { RunsOnRow, RunsOnServed } from "./runs-on-model";

/* Copy the Devices cell, its popover and the strip share (`events.*`). */

export const versionText = (version: readonly number[]) => version.join(".");

export const SEPARATOR = (t: DevicesT) =>
	t("devices:events.summary.separator", " · ");

export interface CantRun {
	/** APP §7.3 short form ("Schedules can't"); for a paused event the whole line. */
	short: string;
	paused: boolean;
	/** One line that stands alone: "Can't run on devices: Schedules can't". */
	sentence: string;
}

/** Why an event can't run on devices, as the Events list says it; null when it can. */
export function cantRun(
	t: DevicesT,
	devices: Pick<EventsDevicesValue, "eligibility" | "events">,
	eventId: string,
): CantRun | null {
	const rule = devices.eligibility.get(eventId);
	if (!rule?.code) return null;
	const { short } = eligibilityCopy(t, {
		code: rule.code,
		eventType: devices.events.get(eventId)?.event_type ?? "",
		...(rule.detail ? { detail: rule.detail } : {}),
	});
	const paused = rule.code === "paused";
	return {
		short,
		paused,
		sentence: paused
			? short
			: t(
					"devices:events.summary.cantRun",
					"Can't run on devices: {{reason}}",
					{ reason: short },
				),
	};
}

type ObservedState = Parameters<typeof enumLabel<"observed">>[2];

/** "Running", "Restarting after a crash": what the device reports for the serving service. */
export function servedActual(t: DevicesT, served: RunsOnServed): string {
	return served.service
		? enumLabel(t, "observed", served.service.view.observed as ObservedState)
		: convergenceLabel(t, served.cell.conv ?? "unknown");
}

export interface NewerPart {
	kind: "event" | "flow";
	/** What the device runs and what the newest version has. */
	from: string;
	to: string;
}

/** What the newest version has that this device doesn't: the event version, the flow version, or both. */
export function newerParts(served: RunsOnServed, row: RunsOnRow): NewerPart[] {
	const { pin } = served.cell;
	if (!served.cell.behind || !pin || !row.pin) return [];
	const parts: NewerPart[] = [
		{
			kind: "event",
			from: versionText(pin.eventVersion),
			to: versionText(row.pin.eventVersion),
		},
		{
			kind: "flow",
			from: versionText(pin.boardVersion),
			to: versionText(row.pin.boardVersion),
		},
	];
	return parts.filter((part) => part.from !== part.to);
}

/** "event 1.5.0", "flow 2.2.0". */
export function versionLabel(
	t: DevicesT,
	kind: NewerPart["kind"],
	version: string,
): string {
	return kind === "event"
		? t("devices:events.eventVersion", "event {{version}}", { version })
		: t("devices:events.flowVersion", "flow {{version}}", { version });
}

const BLOCK_GATE: Record<EventsDevicesBlock, GateKind> = {
	signed_out: "noaccess",
	token: "noaccess",
	hub_off: "hub",
	blind: "role",
	error: "hub",
	no_target: "live",
};

export function blockGateKind(block: EventsDevicesBlock): GateKind {
	return BLOCK_GATE[block];
}

/** The one-line reason next to a disabled Run on a device… or Deploy to devices…. */
export function blockShort(t: DevicesT, block: EventsDevicesBlock): string {
	const lines = {
		signed_out: t(
			"devices:events.block.signedOut.short",
			"Sign in to deploy to devices",
		),
		token: t(
			"devices:events.block.token.short",
			"Your access token can't manage devices",
		),
		hub_off: t(
			"devices:events.block.hubOff.short",
			"Device status off on this hub",
		),
		blind: t(
			"devices:events.block.blind.short",
			"Your role can't read this app's flows",
		),
		error: t("devices:events.block.error.short", "Device status unavailable"),
		no_target: t(
			"devices:events.block.noTarget.short",
			"No device can take a deploy right now",
		),
	} satisfies Record<EventsDevicesBlock, string>;
	return lines[block];
}
