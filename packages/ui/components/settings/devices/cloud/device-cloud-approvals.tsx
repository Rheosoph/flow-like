"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Cloud, RefreshCw, Sparkles } from "lucide-react";
import { type ReactNode, useState } from "react";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StateView } from "../primitives/state-view";
import type { DeviceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy } from "../workspace";
import {
	ActionResults,
	RevokeApprovalButton,
	RevokeLimitButton,
} from "./action-controls";
import type { CloudApproval, CloudLimit } from "./cloud-model";
import {
	AppRef,
	ApprovalChip,
	CloudPerson,
	EndRow,
	ModelList,
	RouteButton,
	RouteLink,
	usePersonName,
} from "./cloud-parts";
import { LeasesTable, leaseNote } from "./leases-table";
import { SpendMeterRow } from "./spend-meter-row";
import {
	type DeviceApprovals,
	type DeviceRef,
	useDeviceApprovals,
	useDeviceRefs,
} from "./use-cloud";
import { useCloudActions } from "./use-cloud-actions";

const CAP = 5;

function AllDevicesLink() {
	const { t } = useTranslation("devices");
	return (
		<RouteLink
			route={{ screen: "access", tab: "cloud" }}
			scope={{ kind: "account" }}
		>
			{t("cloud.device.all", "Cloud approvals and spending on all devices")}
		</RouteLink>
	);
}

/** "paid by you · ends Oct 29": who pays and until when, after the amounts. */
function usePaidBy(limit: CloudLimit | undefined): string | undefined {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const name = usePersonName(limit?.payerId);
	if (!limit) return undefined;
	const values = { name, date: time.at(limit.expiresAt) };
	if (limit.state !== "active")
		return limit.payerIsMe
			? t("cloud.device.paidByYouEnded", "paid by you")
			: name
				? t("cloud.device.paidByEnded", "paid by {{name}}", values)
				: undefined;
	return limit.payerIsMe
		? t("cloud.device.paidByYou", "paid by you · ends {{date}}", values)
		: name
			? t("cloud.device.paidBy", "paid by {{name}} · ends {{date}}", values)
			: t(
					"cloud.device.paidBySomeone",
					"paid by someone else · ends {{date}}",
					values,
				);
}

function ApprovalBlock({
	approval,
	device,
	stamp,
}: Readonly<{ approval: CloudApproval; device: DeviceRef; stamp: ReactNode }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { serviceId, deviceId, details, limit, leases } = approval;
	const modelOnly = approval.appId === null;
	const actions = useCloudActions({
		deviceId,
		deviceName: device.name,
		serviceId,
		approval,
		modelOnly,
		sub: t("cloud.row.context", "{{service}} · on {{device}}", {
			service: serviceId,
			device: device.name,
		}),
	});
	const limitLive = limit?.state === "active";
	const paidBy = usePaidBy(limit);
	const revokeApproval = approval.revocable ? (
		<RevokeApprovalButton actions={actions} />
	) : null;
	const revokeLimit = limitLive ? (
		<RevokeLimitButton actions={actions} />
	) : null;
	// A control that can't be used goes last, so its reason doesn't push the usable one away.
	const limitFirst = !!actions.revokeApprovalGate && !actions.revokeLimitGate;
	return (
		<Block
			id={`device-cloud-${serviceId}`}
			icon={modelOnly ? Sparkles : Cloud}
			title={
				modelOnly ? (
					<Trans
						t={t}
						i18nKey="cloud.device.titleModel"
						defaults="Model access · <1>{{service}}</1>"
						values={{ service: serviceId }}
						components={{ 1: <span className="font-mono" /> }}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="cloud.device.title"
						defaults="Cloud approval · <1>{{service}}</1>"
						values={{ service: serviceId }}
						components={{ 1: <span className="font-mono" /> }}
					/>
				)
			}
			stamp={stamp}
			foot={<AllDevicesLink />}
		>
			{device.revoked && limit && limitLive && limit.payerIsMe ? (
				<Banner
					tone="warning"
					title={t(
						"cloud.device.stillPaying",
						"You still pay for a revoked device.",
					)}
				>
					{t(
						"cloud.device.stillPayingText",
						"{{device}} can't run anything any more, but this spending limit stays active until {{date}} unless you revoke it.",
						{ device: device.name, date: time.at(limit.expiresAt) },
					)}
				</Banner>
			) : null}
			<KeyValueList>
				<KvRow label={t("cloud.kv.service", "Service")}>
					<span className="font-mono">{serviceId}</span>
					<span className="text-muted-foreground"> · </span>
					<AppRef
						appId={approval.appId}
						{...(details ? { projectId: details.projectId } : {})}
					/>
				</KvRow>
				<KvRow label={t("cloud.kv.status", "Status")}>
					<ApprovalChip state={approval.state} />
				</KvRow>
				{details?.approvedBy ? (
					<KvRow label={t("cloud.kv.approvedBy", "Approved by")}>
						<CloudPerson userId={details.approvedBy} />
						{details.approvedAt ? (
							<span className="ml-2 text-muted-foreground">
								{t("cloud.kv.approvedAt", "approved {{date}}", {
									date: time.at(details.approvedAt),
								})}
							</span>
						) : null}
					</KvRow>
				) : null}
				<KvRow label={t("cloud.kv.models", "Models")}>
					{details ? (
						<ModelList ids={details.models} />
					) : (
						<span className="text-muted-foreground">
							{t(
								"cloud.kv.hidden",
								"Only the approver and the device owner see this.",
							)}
						</span>
					)}
				</KvRow>
				{modelOnly ? null : (
					<KvRow label={t("cloud.kv.files", "Project files")}>
						{enumLabel(t, "onlineAccess", approval.files)}
						{approval.writeBlocked ? (
							<span
								data-write-blocked=""
								className="mt-0.5 block text-xs text-warning"
							>
								{t(
									"cloud.row.writeBlocked",
									"Read-only right now: project storage is full",
								)}
							</span>
						) : null}
					</KvRow>
				)}
				{details ? (
					<KvRow label={t("cloud.kv.instances", "Instances allowed")}>
						<span className="tabular-nums">{details.maxInstances}</span>
					</KvRow>
				) : null}
				<EndRow approval={approval} deviceName={device.name} />
			</KeyValueList>
			{limit ? (
				<SpendMeterRow
					limit={limit}
					end={!limitLive}
					meterClassName="max-w-70"
					{...(paidBy ? { tail: paidBy } : {})}
				/>
			) : details?.models.length ? (
				<p data-no-limit="" className="text-xs text-warning">
					{t(
						"cloud.device.noLimit",
						"No spending limit: hosted models refuse its model calls until someone sets one.",
					)}
				</p>
			) : null}
			{leases?.length ? (
				<div className="flex min-w-0 flex-col gap-1.5">
					<div className="max-w-140 min-w-0">
						<LeasesTable leases={leases} compact />
					</div>
					<p className="text-xs text-muted-foreground">{leaseNote(t)}</p>
				</div>
			) : null}
			<div className="flex flex-wrap items-start gap-2">
				{device.revoked ? null : (
					<RouteButton
						data-act="open-service"
						route={{ screen: "service", deviceId, serviceId, tab: "cloud" }}
					>
						{modelOnly
							? t("cloud.device.openModel", "Open service's model access")
							: t("cloud.device.open", "Open service's cloud access")}
					</RouteButton>
				)}
				{limitFirst ? revokeLimit : revokeApproval}
				{limitFirst ? revokeApproval : revokeLimit}
			</div>
			<ActionResults actions={actions} />
		</Block>
	);
}

function Unloaded({
	approvals,
	stamp,
}: Readonly<{ approvals: DeviceApprovals; stamp: ReactNode }>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			id="device-cloud"
			icon={Cloud}
			title={t("cloud.device.heading", "Cloud approvals")}
			stamp={stamp}
			foot={<AllDevicesLink />}
		>
			{approvals.error ? (
				<StateView
					kind="error"
					title={t(
						"cloud.device.failed",
						"Couldn't read this device's cloud approvals",
					)}
					text={t(
						"cloud.device.failedText",
						"{{why}} Whether services here have cloud access isn't known until this can be read.",
						{ why: hubErrorCopy(t, approvals.error.code) },
					)}
					actions={
						<DvButton
							size="sm"
							icon={RefreshCw}
							onClick={() => void approvals.refetch()}
						>
							{t("cloud.retry", "Try again")}
						</DvButton>
					}
				/>
			) : (
				<StateView
					kind="loading"
					title={t("cloud.device.loading", "Reading cloud approvals…")}
				/>
			)}
		</Block>
	);
}

/** SPEC §5.2 Access › Cloud approvals: every approval on one device, also when the device is revoked. */
export function DeviceCloudApprovals({ deviceId }: Readonly<DeviceTabProps>) {
	const { t } = useTranslation("devices");
	const approvals = useDeviceApprovals(deviceId);
	const device = useDeviceRefs()(deviceId);
	const [cap, setCap] = useState(CAP);
	const stamp = <FreshnessStamp {...stampOf(approvals.freshness)} />;

	if (!approvals.loaded && !approvals.rows.length)
		return <Unloaded approvals={approvals} stamp={stamp} />;

	if (!approvals.rows.length)
		return (
			<Block
				id="device-cloud"
				icon={Cloud}
				title={t("cloud.device.heading", "Cloud approvals")}
				stamp={stamp}
				foot={<AllDevicesLink />}
			>
				<p data-cloud-empty="" className="text-ui text-muted-foreground">
					{device.revoked
						? t(
								"cloud.device.emptyRevoked",
								"No cloud approvals or spending limits are left on {{device}}.",
								{ device: device.name },
							)
						: t(
								"cloud.device.empty",
								"No cloud approvals on {{device}}. Services here use local resources only.",
								{ device: device.name },
							)}
				</p>
			</Block>
		);

	const shown = approvals.rows.slice(0, cap);
	const rest = approvals.rows.length - shown.length;
	return (
		<div data-device-cloud="" className="flex min-w-0 flex-col gap-4">
			{shown.map((approval) => (
				<ApprovalBlock
					key={approval.grantId}
					approval={approval}
					device={device}
					stamp={stamp}
				/>
			))}
			{rest > 0 ? (
				<DvButton
					size="sm"
					variant="ghost"
					className="self-start"
					data-act="show-more"
					onClick={() => setCap((current) => current + CAP)}
				>
					{t("cloud.device.showMore", {
						count: rest,
						defaultValue_one: "Show {{count, number}} more approval",
						defaultValue_other: "Show {{count, number}} more approvals",
					})}
				</DvButton>
			) : null}
		</div>
	);
}
