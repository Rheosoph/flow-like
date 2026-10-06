"use client";

import { useTranslation } from "@flow-like/locales";
import { Lock } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { toast } from "sonner";
import type {
	ModelJob,
	ModelsOverview,
} from "../../../../lib/device-management/models";
import { OBSERVE_STACK } from "../observe/observe-data";
import {
	type ObserveTarget,
	useObserveTarget,
} from "../observe/use-observe-target";
import { StateView } from "../primitives/state-view";
import { DvButton } from "../primitives/dv-button";
import type { DeviceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { useKeyChip } from "../workspace/use-keys";
import { DownloadsBlock } from "./downloads-block";
import { HardwareBlock } from "./hardware-block";
import { ModelsHeadline, RecommendationsBlock } from "./models-overview";
import {
	ModelsGateState,
	ModelsReadFailed,
	ModelsSnapshotBlock,
	OlderAgentNotice,
} from "./models-states";
import { ModelsBlock } from "./models-table";
import { ModelStatsBlock } from "./stats/stats-block";
import {
	type ModelJobs,
	type ModelsAccess,
	type ModelsRead,
	useModelsAccess,
	useModelsAfter,
	useModelsJobs,
	useModelsOverview,
	useModelsRecommendations,
	withAllModels,
	withAllRecommendations,
} from "./use-models";

export interface ModelsTabSlots {
	/** Lane C1: "Send from this computer" under a download the device can't finish itself. */
	sendFromThisComputer?: (job: ModelJob) => ReactNode;
}

export interface DeviceModelsTabProps extends DeviceTabProps, ModelsTabSlots {}

function LockAllDevices() {
	const { t } = useTranslation("devices");
	const keys = useKeyChip();
	return (
		<div className="flex justify-end">
			<DvButton
				size="sm"
				variant="ghost"
				icon={Lock}
				aria-disabled={keys.unlockedCount === 0 || undefined}
				onClick={() => {
					keys.lockAll();
					toast(
						t("devices:chrome.keys.lockedAllToast", "All devices are locked on this computer."),
					);
				}}
			>
				{t("devices:models.lockAll", "Lock all devices")}
			</DvButton>
		</div>
	);
}

interface ContentProps extends ModelsTabSlots {
	target: ObserveTarget;
	overview: ModelsRead<ModelsOverview> & { data: ModelsOverview };
	jobs: ModelsRead<ModelJobs>;
}

/** Headline, recommendations, models, usage, downloads, hardware: plan §3.7 in order. */
function ModelsContent({
	target,
	overview,
	jobs,
	sendFromThisComputer,
}: Readonly<ContentProps>) {
	const stamp = stampOf(overview.freshness);
	const after = useModelsAfter(target.deviceId, overview.data.next);
	const recommendations = useModelsRecommendations(
		target.deviceId,
		overview.data,
	);
	const data = useMemo(
		() =>
			withAllRecommendations(
				withAllModels(overview.data, after.data),
				recommendations.data,
			),
		[overview.data, after.data, recommendations.data],
	);
	return (
		<>
			<ModelsHeadline overview={data} device={target.name} />
			<RecommendationsBlock
				deviceId={target.deviceId}
				overview={data}
				device={target.name}
				stamp={stamp}
			/>
			<ModelsBlock target={target} overview={data} stamp={stamp} />
			<ModelStatsBlock target={target} overview={data} />
			<DownloadsBlock
				deviceId={target.deviceId}
				device={target.name}
				jobs={jobs}
				sendFromThisComputer={sendFromThisComputer}
			/>
			<HardwareBlock deviceId={target.deviceId} overview={data} stamp={stamp} />
		</>
	);
}

interface BodyProps extends ModelsTabSlots {
	target: ObserveTarget;
	access: ModelsAccess;
	overview: ModelsRead<ModelsOverview>;
	jobs: ModelsRead<ModelJobs>;
}

/** Last data first (it stays with its age while the session is down), then why nothing can be read. */
function ModelsBody({
	target,
	access,
	overview,
	...rest
}: Readonly<BodyProps>) {
	const { data } = overview;
	if (data)
		return (
			<ModelsContent
				target={target}
				overview={{ ...overview, data }}
				{...rest}
			/>
		);
	return (
		<>
			<ModelsUnread target={target} access={access} overview={overview} />
			<ModelsSnapshotBlock target={target} />
		</>
	);
}

/** Why nothing was read live yet: the device's last reported counts follow it. */
function ModelsUnread({
	target,
	access,
	overview,
}: Readonly<Pick<BodyProps, "target" | "access" | "overview">>) {
	const { t } = useTranslation("devices");
	if (overview.unsupported) return <OlderAgentNotice target={target} />;
	if (!access.gate.ok)
		return <ModelsGateState target={target} gate={access.gate} />;
	if (overview.failure)
		return (
			<ModelsReadFailed
				target={target}
				failure={overview.failure}
				onRetry={overview.refetch}
			/>
		);
	return (
		<StateView
			kind="loading"
			rows={6}
			title={t(
				"devices:models.states.reading",
				"Reading models from {{device}}…",
				{
					device: target.name,
				},
			)}
		/>
	);
}

/** N2 › Models (plan §3.7): what the device hosts, how it serves it and what it runs on. Whole device. */
export function DeviceModelsTab({
	deviceId,
	scope,
	sendFromThisComputer,
}: Readonly<DeviceModelsTabProps>) {
	const { t } = useTranslation("devices");
	const target = useObserveTarget(deviceId, null, scope);
	const access = useModelsAccess(deviceId);
	const overview = useModelsOverview(deviceId);
	const jobs = useModelsJobs(deviceId);
	return (
		<div className={OBSERVE_STACK} data-models="">
			<LockAllDevices />
			{target.known ? (
				<ModelsBody
					target={target}
					access={access}
					overview={overview}
					jobs={jobs}
					sendFromThisComputer={sendFromThisComputer}
				/>
			) : (
				<StateView
					kind="loading"
					title={t("devices:models.states.loading", "Loading the device…")}
				/>
			)}
		</div>
	);
}
