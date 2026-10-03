"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleSlash,
	History,
	type LucideIcon,
	OctagonX,
	RotateCcw,
} from "lucide-react";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import type { ServiceView } from "../../../../lib/device-management/model/types";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import {
	type ServiceRollouts,
	rolloutEndedAt,
	rolloutOutcome,
	rolloutPhase,
	rolloutSettings,
} from "./current-update";

const FINISHED = new Set<DeploymentRolloutStatus["state"]>([
	"healthy",
	"rolled_back",
	"failed",
	"cancelled",
]);

interface Look {
	tone: ChipTone;
	icon: LucideIcon;
}

const LOOK: Record<string, Look> = {
	healthy: { tone: "good", icon: CircleCheck },
	rolled_back: { tone: "warning", icon: RotateCcw },
	not_applied: { tone: "warning", icon: CircleSlash },
	failed_stopped: { tone: "critical", icon: OctagonX },
	cancelled: { tone: "outline", icon: CircleSlash },
};

function historyLabel(t: DevicesT, rollout: DeploymentRolloutStatus) {
	if (rollout.state === "cancelled" && rollout.failure_code === "discarded")
		return t("devices:service.history.discarded", "Discarded");
	return enumLabel(t, "rollout", rollout.state, {
		failureCode: rollout.failure_code ?? "",
	});
}

function historySentence(
	t: DevicesT,
	rollout: DeploymentRolloutStatus,
	service: ServiceView,
) {
	const { to } = rolloutSettings(t, rollout, service);
	if (rollout.state === "healthy")
		return t(
			"devices:service.history.updated",
			"Safe update to settings v{{to}}. It stayed healthy for {{count}} s after the new version was ready.",
			{ to, count: rollout.stabilization_seconds ?? 10 },
		);
	if (rollout.state === "cancelled" && rollout.failure_code === "discarded")
		return t(
			"devices:service.history.discardedText",
			"Staged update discarded before switching over.",
		);
	return (
		rolloutOutcome(t, rollout) ??
		t(
			"devices:service.history.noReason",
			"The device recorded no reason for this result.",
		)
	);
}

/** SPEC §5.3 Status › Update history (BG12): finished updates, newest first. */
export function UpdateHistory({
	deviceName,
	service,
	rollouts,
	live,
	offline,
	lockedAt,
}: Readonly<{
	deviceName: string;
	service: ServiceView;
	rollouts: ServiceRollouts;
	/** A live session is open: the history can be read now. */
	live: boolean;
	offline: boolean;
	/** Unix seconds the rows were read before the keys locked. */
	lockedAt?: number;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const rows = rollouts.rollouts.filter((row) => FINISHED.has(row.state));
	const latestOnly = rollouts.coverage === "latest";
	const shown = latestOnly ? rows.slice(0, 1) : rows;

	const stamp =
		lockedAt !== undefined ? (
			<FreshnessStamp source="live" age="locked" observedAt={lockedAt} />
		) : live ? (
			<FreshnessStamp
				source="live"
				age={rollouts.failed ? "error" : "live"}
				text={
					rollouts.failed
						? undefined
						: t("service.history.onDemand", "read on demand")
				}
				{...(rollouts.failed && rollouts.readAt
					? { error: { dataFrom: rollouts.readAt } }
					: {})}
			/>
		) : (
			<FreshnessStamp
				source="live"
				age="notloaded"
				text={t("service.history.needsLive", "needs a live connection")}
			/>
		);

	const hint = latestOnly
		? t(
				"service.history.latestOnly",
				"Only the latest finished update is shown. The device keeps the last 32; update its agent to list them here.",
			)
		: t(
				"service.history.keeps",
				"The device keeps the last 32 finished updates.",
			);

	const body = () => {
		if (shown.length)
			return (
				<>
					<ul className="flex flex-col">
						{shown.map((row) => {
							const phase = rolloutPhase(row) ?? "cancelled";
							const look = LOOK[phase] ?? LOOK.cancelled;
							const at = rolloutEndedAt(row);
							return (
								<li
									key={row.rollout_id}
									data-history={phase}
									className="flex flex-col gap-1 border-t border-hairline py-2 text-ui first:border-t-0 first:pt-0"
								>
									<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
										<span
											className="font-mono text-xs text-muted-foreground tabular-nums"
											title={at === undefined ? undefined : time.abs(at)}
										>
											{at === undefined
												? t("service.history.noDate", "Date unknown")
												: time.at(at)}
										</span>
										<StatusChip tone={look.tone} icon={look.icon}>
											{historyLabel(t, row)}
										</StatusChip>
									</span>
									<span>{historySentence(t, row, service)}</span>
								</li>
							);
						})}
					</ul>
					<p className="text-xs text-muted-foreground">{hint}</p>
					{rollouts.more ? (
						<div>
							<DvButton
								size="sm"
								variant="ghost"
								busy={rollouts.loading}
								onClick={rollouts.loadOlder}
							>
								{t("service.history.older", "Show older updates")}
							</DvButton>
						</div>
					) : null}
				</>
			);
		if (!live && lockedAt === undefined)
			return (
				<StateView
					kind="notloaded"
					title={t("service.history.needsLiveTitle", "Needs a live connection")}
					text={
						offline
							? t(
									"service.history.needsLiveOffline",
									"Update history is read live from {{device}}, which is offline.",
									{ device: deviceName },
								)
							: t(
									"service.history.needsLiveText",
									"Update history is read live from {{device}}.",
									{ device: deviceName },
								)
					}
				/>
			);
		if (rollouts.coverage === "unread")
			return rollouts.failed ? (
				<StateView
					kind="error"
					title={t(
						"service.history.failed",
						"The update history couldn't be read",
					)}
					actions={
						<DvButton size="sm" onClick={rollouts.refresh}>
							{t("service.history.retry", "Try again")}
						</DvButton>
					}
				/>
			) : (
				<StateView kind="loading" rows={2} />
			);
		return (
			<p className="text-ui text-muted-foreground">
				{latestOnly
					? t(
							"service.history.noneKnown",
							"No finished update is known on this computer. The device keeps the last 32; update its agent to list them here.",
						)
					: t("service.history.none", "No finished updates yet.")}
			</p>
		);
	};

	return (
		<Block
			icon={History}
			title={t("service.history.title", "Update history")}
			stamp={stamp}
		>
			{body()}
		</Block>
	);
}
