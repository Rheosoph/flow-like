"use client";

import { useTranslation } from "@flow-like/locales";
import { CirclePlay } from "lucide-react";
import { useMemo } from "react";
import type { ServiceView } from "../../../../lib/device-management/model/types";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { formTakesFile } from "../service/service-events";
import { type GateTarget, serviceGateExtra, useGate } from "../workspace";
import {
	type RunNowRequest,
	useOverlayStore,
} from "../workspace/overlay-store";
import { useGateText } from "./app-shared";

/* "Run now…" for a person-started event that a service runs (design R2 §6.5): one control for every entry point. */

export type RunNowTarget = Pick<
	RunNowRequest,
	"deviceId" | "serviceId" | "eventId"
>;

/** What `run_event` is checked against for one service. */
export function runNowGateTarget(view: ServiceView): GateTarget {
	return {
		placementId: view.serviceId,
		projectId: view.projectId,
		labels: { service: view.serviceId },
		extra: serviceGateExtra(view),
	};
}

/** Run now…, or the reason it is off (locked, not connected, no Start, not running, agent too old). */
export function RunNowButton({
	gate,
	onOpen,
	label,
}: Readonly<{
	gate: Gate | null;
	onOpen(): void;
	/** With the event's name where the row doesn't say which event it runs. */
	label?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<GatedAction gate={gate}>
			<DvButton size="xs" icon={CirclePlay} data-run-now="" onClick={onOpen}>
				{label ?? t("events.pop.runNow", "Run now…")}
			</DvButton>
		</GatedAction>
	);
}

/** The `run_event` gate of one service in the Devices area, as the reason a control shows. */
export function useRunNowGate(
	deviceId: string,
	view: ServiceView,
): Gate | null {
	const target = useMemo(() => runNowGateTarget(view), [view]);
	const result = useGate("run_event", deviceId, target);
	const gateText = useGateText();
	return result.ok || result.hide ? null : gateText(result);
}

/** Opens the "Run now…" sheet of one event; the area's overlay host renders it, also under the Events page. */
export function openRunNow(target: RunNowTarget): void {
	useOverlayStore.getState().openRunNow(target);
}

/** Run now… for one person-started event of a service in the Devices area; a form that takes a file is off. */
export function RunNowAction({
	deviceId,
	view,
	eventId,
}: Readonly<{ deviceId: string; view: ServiceView; eventId: string }>) {
	const { t } = useTranslation("devices");
	const gate = useRunNowGate(deviceId, view);
	const file: Gate | null = formTakesFile(view, eventId)
		? {
				kind: "unsupported",
				reason: t(
					"serviceStatus.actions.fileOnly",
					"This form takes a file. Open it on the service page.",
				),
			}
		: null;
	return (
		<RunNowButton
			gate={file ?? gate}
			onOpen={() =>
				openRunNow({ deviceId, serviceId: view.serviceId, eventId })
			}
		/>
	);
}
