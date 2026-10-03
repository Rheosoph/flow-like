"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { OctagonX, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { CellSub, Td, Tr } from "../primitives/dv-table";
import { IdRef } from "../primitives/id-ref";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import {
	ActionResults,
	RevokeApprovalButton,
	RevokeLimitButton,
	useActionResults,
} from "./action-controls";
import type { CloudApproval } from "./cloud-model";
import {
	AppRef,
	ApprovalChip,
	CloudPerson,
	ModelList,
	NAME_LINK,
	NAME_TEXT,
	Payer,
	RouteButton,
	RouteLink,
	useExpiryCopy,
} from "./cloud-parts";
import { SpendMeterRow } from "./spend-meter-row";
import type { DetailState, DeviceRef } from "./use-cloud";
import { type CloudActions, useCloudActions } from "./use-cloud-actions";

export const APPROVAL_COLUMNS = [
	"17%",
	"15%",
	"5%",
	"10%",
	"12%",
	"16%",
	"8%",
	"17%",
] as const;

export interface ApprovalColumnLabels {
	service: string;
	models: string;
	/** The short column head; stacked cards use `instances`. */
	max: string;
	instances: string;
	expires: string;
	status: string;
	spending: string;
	paidBy: string;
	actions: string;
}

/** Models and instances are only on the device's own list; say why they are missing. */
function Hidden({ detail }: Readonly<{ detail: DetailState }>) {
	const { t } = useTranslation("devices");
	return (
		<span data-detail={detail} className="text-muted-foreground">
			{detail === "loading"
				? t("cloud.row.reading", "Reading…")
				: detail === "failed"
					? t("cloud.row.failed", "Couldn't be read")
					: t(
							"cloud.row.hidden",
							"Only the approver and the device owner see this",
						)}
		</span>
	);
}

function StatusCell({
	approval,
	device,
}: Readonly<{ approval: CloudApproval; device: DeviceRef }>) {
	const { t } = useTranslation("devices");
	const billed = approval.limit?.state === "active" && approval.limit.payerIsMe;
	const blocked = approval.writeBlocked ? (
		<CellSub data-write-blocked="" className="text-warning">
			{t(
				"cloud.row.writeBlocked",
				"Read-only right now: project storage is full",
			)}
		</CellSub>
	) : null;
	if (!device.revoked)
		return (
			<>
				<ApprovalChip state={approval.state} />
				{blocked}
			</>
		);
	const state = {
		active: t("cloud.row.approvalActive", "Approval still active"),
		revoked: t("cloud.row.approvalRevoked", "Approval revoked"),
		expired: t("cloud.row.approvalEnded", "Approval ended"),
	}[approval.state];
	return (
		<>
			<StatusChip tone="critical" icon={OctagonX}>
				{t("cloud.row.deviceRevoked", "Device revoked")}
			</StatusChip>
			<CellSub>
				{billed
					? t("cloud.row.stillBilled", "{{state}} · still billed to you", {
							state,
						})
					: state}
			</CellSub>
		</>
	);
}

function SpendingCell({
	approval,
	device,
}: Readonly<{ approval: CloudApproval; device: DeviceRef }>) {
	const { t } = useTranslation("devices");
	const limit = approval.limit;
	if (limit)
		return (
			<SpendMeterRow limit={limit}>
				{device.revoked && limit.state === "active" && limit.payerIsMe ? (
					<span
						data-still-paying=""
						className="mt-1 flex items-start gap-1 text-xs text-warning"
					>
						<TriangleAlert aria-hidden className="mt-px size-3.5 shrink-0" />
						{t("cloud.row.stillPaying", "You still pay for a revoked device.")}
					</span>
				) : null}
			</SpendMeterRow>
		);
	const models = approval.details?.models.length;
	return (
		<>
			<span className="text-muted-foreground">
				{t("cloud.row.noLimit", "No spending limit")}
			</span>
			{models === 0 ? (
				<CellSub>
					{t("cloud.row.noLimitFree", "Nothing on this approval is billed.")}
				</CellSub>
			) : models ? (
				<CellSub className="text-warning">
					{t(
						"cloud.row.noLimitRefused",
						"Hosted models refuse its model calls until someone sets one.",
					)}
				</CellSub>
			) : null}
		</>
	);
}

function Actions({
	approval,
	device,
	actions,
}: Readonly<{
	approval: CloudApproval;
	device: DeviceRef;
	actions: CloudActions;
}>) {
	const { t } = useTranslation("devices");
	const controls: ReactNode[] = [];
	if (device.row && !device.revoked)
		controls.push(
			<RouteButton
				key="open"
				size="sm"
				data-act="open-service"
				route={{
					screen: "service",
					deviceId: approval.deviceId,
					serviceId: approval.serviceId,
					tab: "cloud",
				}}
			>
				{t("cloud.row.open", "Open service")}
			</RouteButton>,
		);
	if (approval.limit?.state === "active")
		controls.push(
			<RevokeLimitButton key="limit" actions={actions} size="sm" />,
		);
	if (approval.revocable)
		controls.push(
			<RevokeApprovalButton key="approval" actions={actions} size="sm" />,
		);
	if (!controls.length)
		return (
			<span className="text-xs text-muted-foreground">
				{t("cloud.row.nothingLeft", "Nothing left to revoke.")}
			</span>
		);
	return <div className="flex flex-col items-start gap-1.5">{controls}</div>;
}

function Details({
	approval,
	detail,
}: Readonly<{ approval: CloudApproval; detail: DetailState }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { details, limit, leases } = approval;
	return (
		<KeyValueList>
			<KvRow label={t("cloud.kv.approvedBy", "Approved by")}>
				{details?.approvedBy ? (
					<>
						<CloudPerson userId={details.approvedBy} fullName />
						{details.approvedAt ? (
							<span className="ml-2 text-muted-foreground">
								{t("cloud.kv.approvedAt", "approved {{date}}", {
									date: time.at(details.approvedAt),
								})}
							</span>
						) : null}
					</>
				) : approval.approverIsMe ? (
					t("cloud.spend.you", "You")
				) : (
					<Hidden detail={detail} />
				)}
			</KvRow>
			<KvRow label={t("cloud.row.approval", "Approval")}>
				<IdRef
					id={approval.grantId}
					copyLabel={t("cloud.reference.copyId", "Copy approval ID")}
				/>
				{details ? (
					<span className="ml-2 text-muted-foreground">
						{t("cloud.row.version", "version {{version, number}}", {
							version: details.version,
						})}
					</span>
				) : null}
			</KvRow>
			{limit ? (
				<KvRow label={t("cloud.spend.title", "Spending limit")}>
					<IdRef
						id={limit.id}
						copyLabel={t(
							"cloud.reference.copyLimitId",
							"Copy spending limit ID",
						)}
					/>
					<span className="ml-2 text-muted-foreground">
						{t("cloud.row.limitEnds", "ends {{date}}", {
							date: time.at(limit.expiresAt),
						})}
					</span>
				</KvRow>
			) : null}
			<KvRow label={t("cloud.leases.title", "Cloud leases")}>
				{leases === undefined ? (
					<Hidden detail={detail} />
				) : leases.length ? (
					<ul className="m-0 flex list-none flex-col gap-1 p-0">
						{leases.map((lease) => (
							<li key={lease.instance_id}>
								<Trans
									t={t}
									i18nKey="cloud.row.lease"
									defaults="{{purpose}} <1/> · lease ends {{time}}"
									values={{
										purpose: enumLabel(t, "instancePurpose", lease.purpose),
										time: time.at(lease.lease_expires_at),
									}}
									components={{
										1: (
											<IdRef
												id={lease.instance_id}
												copyLabel={t(
													"cloud.leases.copyInstance",
													"Copy instance ID",
												)}
											/>
										),
									}}
								/>
							</li>
						))}
					</ul>
				) : (
					<span className="text-muted-foreground">
						{t("cloud.row.noLease", "No instance holds a lease right now.")}
					</span>
				)}
			</KvRow>
		</KeyValueList>
	);
}

/** One approval in the Access › Cloud approvals & spending table (SPEC §5.7). */
export function ApprovalRow({
	approval,
	device,
	detail,
	labels,
	expanded,
	onToggle,
	approvedDates = false,
}: Readonly<{
	approval: CloudApproval;
	device: DeviceRef;
	detail: DetailState;
	labels: ApprovalColumnLabels;
	expanded: boolean;
	onToggle(): void;
	/** Older hub: the table says once that ends are the approved dates, so rows don't repeat it. */
	approvedDates?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const expiry = useExpiryCopy();
	const { serviceId, deviceId, details } = approval;
	const actions = useCloudActions({
		deviceId,
		deviceName: device.name,
		serviceId,
		approval,
		modelOnly: approval.appId === null,
		sub: t("cloud.row.context", "{{service}} · on {{device}}", {
			service: serviceId,
			device: device.name,
		}),
	});
	const results = useActionResults(actions);
	const end = expiry(approval, device.name);
	const linked = !!device.row && !device.revoked;
	return (
		<>
			<Tr data-approval={approval.grantId} data-state={approval.state}>
				<Td label={labels.service} kind="name">
					{linked ? (
						<RouteLink
							className={NAME_LINK}
							route={{ screen: "service", deviceId, serviceId, tab: "cloud" }}
						>
							{serviceId}
						</RouteLink>
					) : (
						<span className={NAME_TEXT}>{serviceId}</span>
					)}
					<CellSub>
						<Trans
							t={t}
							i18nKey="cloud.row.onDevice"
							defaults="on <1/>"
							components={{
								1: device.row ? (
									<RouteLink
										className="font-mono underline decoration-border-strong underline-offset-2 hover:decoration-foreground"
										route={{ screen: "device", deviceId, tab: "access" }}
									>
										{device.name}
									</RouteLink>
								) : (
									<IdRef
										id={deviceId}
										copyLabel={t("cloud.row.copyDevice", "Copy device ID")}
									/>
								),
							}}
						/>
					</CellSub>
					<CellSub>
						<AppRef
							appId={approval.appId}
							{...(details ? { projectId: details.projectId } : {})}
						/>
					</CellSub>
					<DvButton
						variant="link"
						size="xs"
						className="mt-0.5"
						data-act="approval-details"
						aria-expanded={expanded}
						onClick={onToggle}
					>
						{expanded
							? t("cloud.row.hideDetails", "Hide details")
							: t("cloud.row.details", "Details")}
					</DvButton>
				</Td>
				<Td label={labels.models}>
					{details ? (
						<ModelList ids={details.models} />
					) : (
						<Hidden detail={detail} />
					)}
					{approval.appId === null ? null : (
						<CellSub>{enumLabel(t, "onlineAccess", approval.files)}</CellSub>
					)}
				</Td>
				<Td label={labels.instances}>
					{details ? (
						<span className="tabular-nums">{details.maxInstances}</span>
					) : (
						<span className="text-muted-foreground">–</span>
					)}
				</Td>
				<Td label={labels.expires}>
					<span data-expiry={approval.effective ? "effective" : "approved"}>
						{end.main}
					</span>
					{end.sub ? <CellSub>{end.sub}</CellSub> : null}
					{end.note && (approval.effective || !approvedDates) ? (
						<CellSub
							className={end.tone === "warning" ? "text-warning" : undefined}
						>
							{end.note}
						</CellSub>
					) : null}
				</Td>
				<Td label={labels.status}>
					<StatusCell approval={approval} device={device} />
				</Td>
				<Td label={labels.spending}>
					<SpendingCell approval={approval} device={device} />
				</Td>
				<Td label={labels.paidBy}>
					<Payer approval={approval} />
				</Td>
				<Td label={labels.actions} kind="act">
					<Actions approval={approval} device={device} actions={actions} />
				</Td>
			</Tr>
			{results.any ? (
				<tr data-approval-result={approval.grantId}>
					<td
						colSpan={APPROVAL_COLUMNS.length}
						className="border-t border-hairline px-4 py-2"
					>
						<div className="flex flex-col gap-1.5">
							<ActionResults actions={actions} />
						</div>
					</td>
				</tr>
			) : null}
			{expanded ? (
				<tr data-approval-details={approval.grantId}>
					<td
						colSpan={APPROVAL_COLUMNS.length}
						className="border-t border-hairline bg-surface-sunken px-4 py-3"
					>
						<Details approval={approval} detail={detail} />
					</td>
				</tr>
			) : null}
		</>
	);
}
