"use client";

import { useTranslation } from "@flow-like/locales";
import { Activity, CircleCheck } from "lucide-react";
import { useCallback } from "react";
import { fleetFacts } from "../../../../lib/device-management/model/device-view";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { TrayItem } from "../primitives/tray-item";
import { useDevicesRoute } from "../routing/use-devices-route";
import {
	activityDetail,
	activityRoute,
	activityTarget,
	activityTitle,
	useActivityTray,
} from "../shell/activity-tray";
import { useActivity, useAttentionInput } from "../workspace";

/** SPEC §5.1: the panel previews at most three operations; the tray has all of them. */
export const IN_PROGRESS_CAP = 3;

function progressOf(item: ActivityItem): number | undefined {
	const { progress } = item;
	if (!progress || progress === "indeterminate" || progress.total <= 0)
		return undefined;
	return Math.min(100, Math.round((progress.done / progress.total) * 100));
}

/** SPEC §5.1 In progress: what this computer is tracking right now, from the activity tray. */
export function InProgressPanel() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { navigate } = useDevicesRoute();
	const { inProgress } = useActivity();
	const input = useAttentionInput();
	const deviceName = useCallback(
		(deviceId: string) => fleetFacts(input).byId.get(deviceId)?.name,
		[input],
	);
	const shown = inProgress.slice(0, IN_PROGRESS_CAP);
	const more = inProgress.length - shown.length;
	return (
		<Block
			icon={Activity}
			title={t("fleet.inProgress.title", "In progress")}
			count={inProgress.length}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("fleet.inProgress.stamp", "tracked on this computer")}
				/>
			}
			flush
			foot={
				<>
					<DvButton
						size="sm"
						icon={Activity}
						onClick={() => useActivityTray.getState().setOpen(true)}
					>
						{t("fleet.inProgress.open", "Open activity")}
					</DvButton>
					{more > 0 ? (
						<span>
							{t(
								"fleet.inProgress.more",
								"{{count, number}} more in Activity",
								{
									count: more,
								},
							)}
						</span>
					) : null}
					<span>
						{t("fleet.inProgress.kept", "Results stay until you dismiss them.")}
					</span>
				</>
			}
		>
			{shown.length ? (
				shown.map((item) => (
					<TrayItem
						key={item.id}
						compact
						kind={item.kind}
						state={item.state}
						title={activityTitle(t, item)}
						sub={activityTarget(item, deviceName)}
						progress={progressOf(item)}
						detail={activityDetail(t, time, item)}
						startedAt={item.startedAt / 1000}
						onOpen={() => {
							const route = activityRoute(item);
							if (route) navigate(route);
						}}
					/>
				))
			) : (
				<p className="flex items-center gap-2 px-4 py-3.5 text-ink-2">
					<CircleCheck aria-hidden className="size-4 text-good" />
					<span>
						{t("fleet.inProgress.none", "Nothing is running right now.")}
					</span>
				</p>
			)}
		</Block>
	);
}
