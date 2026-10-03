"use client";

import { useTranslation } from "@flow-like/locales";
import { holdsSchedule } from "../../../../lib/device-management/model/schedule-where";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import { blockShort, cantRun, heldText } from "./events-copy";
import { type EventsDevicesLink, useEventsDevices } from "./events-devices";
import { EVENTS_BLOCK_ID } from "./on-devices-strip";
import { runsOnExplains, runsOnReasonId } from "./runs-on-cell";

export interface RunOnDevice {
	/** "Run on a device…", or "Run on another device…" once a device serves the event. */
	label: string;
	/** The same with the event's name, for a control that shows only an icon. */
	name: string;
	/** Why it is off, and the id of the element that shows that reason (R7). */
	off: { reason: string; by: string } | null;
	link: EventsDevicesLink;
	open(): void;
	/** The event can run on devices. */
	eligible: boolean;
	/** The Devices cell has a popover: where it runs, or why it can't. */
	explains: boolean;
	/** Opens that popover. */
	show(): void;
	/** A deep link or the row menu last asked for this event. */
	targeted: boolean;
}

/**
 * APP §4.5: Run on a device… for one event of the Events list. Off with the
 * page's block cause, else with the event's own reason from its Devices cell.
 */
export function useRunOnDevice(eventId: string): RunOnDevice {
	const { t } = useTranslation("devices");
	const devices = useEventsDevices();
	const event = devices.events.get(eventId)?.name ?? "";
	const cant = cantRun(t, devices, eventId);
	const row = devices.live?.rows.get(eventId);
	const served = Boolean(row?.served.length);
	const route: DevicesRoute = {
		screen: "deploy",
		deviceIds: [],
		mode: "new",
		eventId,
		from: "events",
	};
	const cellId = runsOnReasonId(eventId);
	const off = devices.block
		? { reason: blockShort(t, devices.block), by: EVENTS_BLOCK_ID }
		: cant
			? { reason: cant.sentence, by: cellId }
			: row && holdsSchedule(row.where)
				? { reason: heldText(t, row), by: cellId }
				: null;
	return {
		label: served
			? t("events.pop.runAnother", "Run on another device…")
			: t("events.cell.run", "Run on a device…"),
		name: served
			? t("events.row.runAnother", "Run {{event}} on another device…", {
					event,
				})
			: t("events.row.run", "Run {{event}} on a device…", { event }),
		off,
		link: devices.link(route),
		open: () => devices.go(route),
		eligible: cant === null,
		explains: runsOnExplains(devices, eventId),
		show: () => devices.showOnDevices(eventId),
		targeted: devices.focus?.eventId === eventId,
	};
}
