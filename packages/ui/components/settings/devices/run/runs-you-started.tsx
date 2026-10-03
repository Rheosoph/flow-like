"use client";

import { useTranslation } from "@flow-like/locales";
import { CirclePlay } from "lucide-react";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { useOverlayStore } from "../workspace/overlay-store";
import { useActivity } from "../workspace/use-activity";
import { cap } from "./run-copy";

/** How long "Runs you started" keeps a run: as long as the device keeps its record. */
export const RUNS_KEPT_MS = 24 * 3_600_000;

export interface RunsYouStartedProps {
	deviceId: string;
	serviceId: string;
	/** The service's events with the names the page knows; a run of an event without one shows its id. */
	events?: readonly { id: string; name?: string }[] | null;
}

function runWord(
	t: DevicesT,
	item: ActivityItem,
): { tone: ChipTone; text: string } {
	switch (item.state) {
		case "done":
			return {
				tone: "good",
				text: t("devices:runNow.started.state.succeeded", "Succeeded"),
			};
		case "failed":
			return item.detail?.code === "cancelled"
				? {
						tone: "critical",
						text: t("devices:runNow.started.state.stopped", "Stopped"),
					}
				: {
						tone: "critical",
						text: t("devices:runNow.started.state.failed", "Didn't succeed"),
					};
		case "unknown":
			return {
				tone: "unknown",
				text: t("devices:runNow.started.state.noReply", "No reply received"),
			};
		case "waiting":
			return {
				tone: "info",
				text: t("devices:runNow.started.state.waiting", "Waiting"),
			};
		default:
			return {
				tone: "info",
				text: t("devices:runNow.started.state.running", "Running"),
			};
	}
}

const operationOf = (item: ActivityItem) =>
	item.resume?.type === "operation" ? item.resume.operationId : undefined;

/** "Runs you started" on a service's Status tab (design R2 §6.5): this computer's runs of its actions and forms for 24 hours. */
export function RunsYouStarted({
	deviceId,
	serviceId,
	events,
}: Readonly<RunsYouStartedProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const runs = useActivity({ deviceId, serviceId, kind: "event_run" });
	const shown = runs.items
		.filter((item) => item.startedAt >= time.now - RUNS_KEPT_MS)
		.sort((a, b) => b.startedAt - a.startedAt);
	if (!shown.length) return null;
	const nameOf = (eventId: string | undefined) => {
		const name = events?.find((event) => event.id === eventId)?.name;
		return name ? cap(name, 120) : (eventId ?? serviceId);
	};
	const show = (item: ActivityItem) => {
		const operationId = operationOf(item);
		const { eventId } = item.target;
		if (operationId && eventId)
			useOverlayStore
				.getState()
				.openRunNow({ deviceId, serviceId, eventId, operationId });
	};
	return (
		<Block
			id="service-runs-you-started"
			icon={CirclePlay}
			title={t("runNow.started.title", "Runs you started")}
			count={shown.length}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("runNow.started.tracked", "tracked here")}
				/>
			}
			foot={t(
				"runNow.started.foot",
				"Runs started from this computer in the last 24 hours. A result is read from the device again when you open it; this computer keeps none.",
			)}
		>
			<ul className="flex flex-col">
				{shown.map((item) => {
					const word = runWord(t, item);
					return (
						<li
							key={item.id}
							data-run-started={item.target.eventId ?? ""}
							data-state={item.state}
							className="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-t border-hairline py-2 text-ui first:border-t-0 first:pt-0"
						>
							<b className="min-w-0 truncate font-semibold">
								{nameOf(item.target.eventId)}
							</b>
							<StatusChip tone={word.tone}>{word.text}</StatusChip>
							<span className="text-xs text-muted-foreground">
								{t("runNow.started.sent", "started {{time}}", {
									time: time.clock(item.startedAt / 1000),
								})}
								{item.finishedAt
									? ` · ${t("runNow.started.finished", "finished {{time}}", {
											time: time.clock(item.finishedAt / 1000),
										})}`
									: ""}
							</span>
							{operationOf(item) && item.target.eventId ? (
								<DvButton
									size="xs"
									variant="ghost"
									className="ml-auto"
									onClick={() => show(item)}
								>
									{item.state === "done" || item.state === "failed"
										? t("runNow.started.show", "Show result…")
										: t("runNow.started.follow", "Follow…")}
								</DvButton>
							) : null}
						</li>
					);
				})}
			</ul>
		</Block>
	);
}
