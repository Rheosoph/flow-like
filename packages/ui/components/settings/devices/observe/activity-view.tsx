"use client";

import { useTranslation } from "@flow-like/locales";
import type { DevicesScope } from "../../../../lib/device-management/model/types";
import { StateView } from "../primitives/state-view";
import { CommandLookup } from "./command-lookup";
import { HistoryAccess } from "./history-access";
import { HistorySettings } from "./history-settings";
import { LogsBlock } from "./logs-block";
import { OBSERVE_COLS, OBSERVE_STACK } from "./observe-data";
import { RetainedHistory } from "./retained-history";
import { TimelineBlock } from "./timeline-block";
import { useHistoryPanel } from "./use-history-panel";
import { useObserveTarget } from "./use-observe-target";

export interface ActivityViewProps {
	deviceId: string;
	/** Absent: the whole device (agent logs, every service's changes). */
	serviceId?: string;
	scope: DevicesScope;
	/** `stream=errors`: the logs open with Errors only and scroll into view. */
	errorsFirst?: boolean;
}

/** Activity & logs of one device or one service (SPEC §5.2, §5.3): one implementation for N2 and N3. */
export function ActivityView({
	deviceId,
	serviceId,
	scope,
	errorsFirst,
}: Readonly<ActivityViewProps>) {
	const { t } = useTranslation("devices");
	const target = useObserveTarget(deviceId, serviceId ?? null, scope);
	const panel = useHistoryPanel(target, true);
	if (!target.known)
		return (
			<StateView
				kind="loading"
				title={t("observe.loadingDevice", "Loading the device…")}
			/>
		);
	const retained = (
		<RetainedHistory
			target={target}
			kind="logs"
			history={panel.history}
			onEdit={panel.onEdit}
		/>
	);
	const settings = target.owner ? (
		<HistorySettings
			target={target}
			history={panel.history}
			scopes={panel.scopes}
			hiddenServices={panel.hiddenServices}
			onEdit={panel.onEdit}
		/>
	) : (
		<HistoryAccess target={target} history={panel.history} />
	);
	return (
		<div className={OBSERVE_STACK} data-observe="activity">
			<div className={OBSERVE_COLS}>
				<TimelineBlock target={target} />
				<div className={OBSERVE_STACK}>
					<LogsBlock target={target} errorsFirst={errorsFirst} />
					<CommandLookup target={target} />
				</div>
			</div>
			{target.revoked ? null : serviceId ? (
				<>
					{settings}
					{retained}
				</>
			) : (
				<>
					{retained}
					{settings}
				</>
			)}
			{panel.sheet}
		</div>
	);
}
