"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleSlash,
	Cloud,
	EyeOff,
	Sparkles,
} from "lucide-react";
import { useMemo } from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type { AppServiceRow } from "../../../../lib/device-management/model/app-plan";
import { revokeDeviceGrant } from "../../../../lib/device-resources";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { SpendMeter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { stampOf } from "../shell/attention-popover";
import {
	useAttentionState,
	useDeviceAction,
	useGates,
	useInlineResults,
} from "../workspace";
import {
	LinkButton,
	Person,
	ShowMore,
	useAppPage,
	useCapped,
	useDeviceNames,
	useGateText,
} from "./app-shared";
import {
	type AppApproval,
	HIDDEN_APPROVAL_CAP,
	ROW_CAP,
	approvalsOf,
} from "./app-view-local";

const resultKeyOf = (approval: AppApproval) =>
	`app-spending:${approval.deviceId}/${approval.serviceId}`;

function useRevoke(approval: AppApproval, row: AppServiceRow) {
	const { t } = useTranslation("devices");
	const { input, workspace } = useAttentionState();
	const actions = useDeviceAction();
	const gateText = useGateText();
	const deviceName = useDeviceNames();
	const { data } = useAppPage();
	const { deviceId, serviceId } = approval;
	const target = useMemo(
		() => ({
			placementId: serviceId,
			projectId: row.view.projectId,
			labels: { service: serviceId },
			extra: {
				approvalExists: true,
				delegatorIsMe: approval.approvedBy === input.me,
				...(approval.billing ? { payerIsMe: approval.billing.payerIsMe } : {}),
			},
		}),
		[serviceId, row.view.projectId, approval, input.me],
	);
	const gates = useGates(
		["cloud_access_revoke", "spending_limit_revoke"],
		deviceId,
		target,
	);
	const resultKey = resultKeyOf(approval);
	const sub = t("app.action.on", "on {{device}}", {
		device: deviceName(deviceId),
	});
	const invalidate = [
		deviceKeys.appPlacements(workspace.scopeKey, data.appId),
		deviceKeys.resources(workspace.scopeKey, deviceId),
		deviceKeys.resourceSummary(workspace.scopeKey),
	];
	const revokeApproval = () =>
		void actions.run({
			action: "cloud_access_revoke",
			deviceId,
			target,
			label: t(
				"app.spending.revokeApprovalLabel",
				"Revoke the cloud access of {{service}}",
				{ service: serviceId },
			),
			consequence: {
				what: t(
					"app.spending.revokeApprovalWhat",
					"{{service}}'s instances lose cloud access within minutes.",
					{ service: serviceId },
				),
				who: t(
					"app.spending.revokeApprovalWho",
					"Model calls and project-file access fail; buffered writes pause and are kept.",
				),
				when: t(
					"app.spending.revokeApprovalWhen",
					"Credentials already issued stay valid for up to 10 minutes. In-flight requests may still complete and bill.",
				),
				undo: {
					reversible: true,
					text: t(
						"app.spending.revokeApprovalUndo",
						"Approve again when you update {{service}}.",
						{ service: serviceId },
					),
				},
			},
			strength: "none",
			confirm: { sub, tone: "danger" },
			resultKey,
			invalidate,
			call: (context) =>
				revokeDeviceGrant(
					context.workspace.hub.api,
					context.workspace.hub.profile,
					deviceId,
					"resource",
					approval.grantId,
				),
		});
	const revokeLimit = () =>
		void actions.run({
			action: "spending_limit_revoke",
			deviceId,
			target,
			label: t(
				"app.spending.revokeLimitLabel",
				"Revoke the spending limit of {{service}}",
				{ service: serviceId },
			),
			consequence: {
				what: t(
					"app.spending.revokeLimitWhat",
					"No new model requests are charged to you.",
				),
				who: t(
					"app.spending.revokeLimitWho",
					"{{service}}'s model calls fail once its credentials expire.",
					{ service: serviceId },
				),
				when: t(
					"app.spending.revokeLimitWhen",
					"Immediately for new requests; credentials already issued last up to 10 minutes.",
				),
				undo: {
					reversible: true,
					text: t("app.spending.revokeLimitUndo", "Add a new limit."),
				},
			},
			strength: "none",
			confirm: { sub, tone: "danger" },
			resultKey,
			invalidate,
			call: (context) =>
				revokeDeviceGrant(
					context.workspace.hub.api,
					context.workspace.hub.profile,
					deviceId,
					"billing",
					approval.billing?.id ?? "",
				),
		});
	return {
		revokeApproval,
		revokeLimit,
		approvalGate: gates.cloud_access_revoke.ok
			? null
			: gateText(gates.cloud_access_revoke),
		limitGate: gates.spending_limit_revoke.ok
			? null
			: gateText(gates.spending_limit_revoke),
		resultKey,
	};
}

function ApprovalRow({
	approval,
	row,
}: Readonly<{ approval: AppApproval; row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const revoke = useRevoke(approval, row);
	const results = useInlineResults(revoke.resultKey);
	const ended = approval.endsAt <= time.nowS;
	const running = approval.active && !ended;
	const billing = approval.billing;
	const files =
		approval.files === "read_write"
			? t("app.spending.filesWrite", "Read & write")
			: approval.files === "read_only"
				? t("app.spending.filesRead", "Read only")
				: t("app.spending.filesNone", "None");
	return (
		<li
			data-approval={`${approval.deviceId}/${approval.serviceId}`}
			className="flex flex-col gap-2.5 border-t border-hairline px-4 py-3 first:border-t-0"
		>
			<div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5">
				<p className="text-ui">
					<Trans
						t={t}
						i18nKey="app.spending.serviceOn"
						defaults="<1/> on <2/>"
						components={{
							1: (
								<b className="font-mono font-semibold">{approval.serviceId}</b>
							),
							2: (
								<span className="font-mono">
									{deviceName(approval.deviceId)}
								</span>
							),
						}}
					/>
				</p>
				{running ? (
					<StatusChip tone="good" icon={CircleCheck}>
						{t("app.spending.active", "Active")}
					</StatusChip>
				) : (
					<StatusChip tone="paused" icon={CircleSlash}>
						{approval.active
							? t("app.spending.ended", "Ended")
							: t("app.spending.revoked", "Revoked")}
					</StatusChip>
				)}
			</div>
			<KeyValueList>
				{approval.approvedBy ? (
					<KvRow label={t("app.spending.approvedBy", "Approved by")}>
						<Person userId={approval.approvedBy} />
					</KvRow>
				) : null}
				<KvRow label={t("app.spending.models", "Models")}>
					{approval.models.length ? (
						<span className="font-mono text-xs">
							{approval.models.join(", ")}
						</span>
					) : (
						t("app.spending.noModels", "None")
					)}
				</KvRow>
				{row.mode === "online" ? (
					<KvRow label={t("app.spending.files", "Project files")}>
						{files}
					</KvRow>
				) : null}
				<KvRow label={t("app.spending.instances", "Instances allowed")}>
					<span className="tabular-nums">{approval.maxInstances}</span>
				</KvRow>
				<KvRow
					label={
						ended
							? t("app.spending.endedAt", "Ended")
							: t("app.spending.ends", "Ends")
					}
				>
					{time.at(approval.endsAt)}
				</KvRow>
			</KeyValueList>
			{billing ? (
				<div className="flex max-w-130 flex-col gap-1">
					<SpendMeter
						used={billing.used / 1_000_000}
						reserved={billing.reserved / 1_000_000}
						limit={billing.limit / 1_000_000}
					/>
					<p className="text-xs text-muted-foreground">
						{billing.payerIsMe
							? t("app.spending.paidByYou", "Paid by you · until {{date}}", {
									date: time.at(billing.endsAt),
								})
							: t(
									"app.spending.paidByOther",
									"Paid by someone else · until {{date}}",
									{ date: time.at(billing.endsAt) },
								)}
					</p>
				</div>
			) : approval.models.length ? (
				<p className="text-xs text-warning">
					{t(
						"app.spending.noLimit",
						"No spending limit, so its model calls are refused.",
					)}
				</p>
			) : null}
			<div className="flex flex-wrap items-start gap-2">
				<LinkButton
					route={{
						screen: "service",
						deviceId: approval.deviceId,
						serviceId: approval.serviceId,
						tab: "cloud",
					}}
					size="sm"
					act="open-cloud"
				>
					{t("app.spending.open", "Open cloud access")}
				</LinkButton>
				{approval.active ? (
					<GatedAction gate={revoke.approvalGate}>
						<DvButton
							size="sm"
							variant="danger-ghost"
							data-act="revoke-approval"
							onClick={revoke.revokeApproval}
						>
							{t("app.spending.revokeApproval", "Revoke approval…")}
						</DvButton>
					</GatedAction>
				) : null}
				{billing ? (
					<GatedAction gate={revoke.limitGate}>
						<DvButton
							size="sm"
							variant="danger-ghost"
							data-act="revoke-limit"
							onClick={revoke.revokeLimit}
						>
							{t("app.spending.revokeLimit", "Revoke spending limit…")}
						</DvButton>
					</GatedAction>
				) : null}
			</div>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</li>
	);
}

/** R11: what a capped list leaves out, with "Show N more". */
function MoreLine({
	shown,
	total,
	onShow,
	hidden = false,
}: Readonly<{
	shown: number;
	total: number;
	onShow(): void;
	/** The rows are approvals the viewer can't see. */
	hidden?: boolean;
}>) {
	const { t } = useTranslation("devices");
	if (shown >= total) return null;
	return (
		<p
			data-more={hidden ? "hidden" : "approved"}
			className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline px-4 py-2 text-xs text-muted-foreground"
		>
			<span>
				{hidden
					? t(
							"app.spending.moreHidden",
							"{{count, number}} more services have cloud access approved by someone else.",
							{ count: total - shown },
						)
					: t(
							"app.spending.moreApproved",
							"Showing {{shown, number}} of {{total, number}} services with cloud access.",
							{ shown, total },
						)}
			</span>
			<ShowMore count={total - shown} onClick={onShow} />
		</p>
	);
}

/** APP §2.13: cloud access & spending of every service (online), or model access (local-only). */
export function AppSpending() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const { input } = useAttentionState();
	const deviceName = useDeviceNames();
	const entries = useMemo(
		() =>
			approvalsOf(
				view,
				(deviceId) => data.devices.get(deviceId)?.resources,
				input.me,
				(deviceId) => data.devices.get(deviceId)?.relationship === "owner",
			),
		[view, data.devices, input.me],
	);
	const approved = entries.filter((entry) => entry.cloud.state === "approved");
	const hidden = entries.filter((entry) => entry.cloud.state === "hidden");
	const unknown = entries.filter((entry) => entry.cloud.state === "unknown");
	const approvedRows = useCapped(approved, ROW_CAP);
	const hiddenRows = useCapped(hidden, HIDDEN_APPROVAL_CAP);
	if (!view.app.canReadFlows) return null;
	const local = view.app.localOnly;
	const stamp = stampOf(data.placements.freshness);
	const interim = data.placements.missingOnHub;
	return (
		<Block
			id="ad-spending"
			icon={local ? Sparkles : Cloud}
			title={
				local
					? t("app.spending.titleLocal", "Model access & spending")
					: t("app.spending.title", "Cloud access & spending")
			}
			{...(approved.length ? { count: approved.length } : {})}
			flush
			stamp={
				interim ? (
					<FreshnessStamp
						source="hub"
						age="current"
						text={t("app.spending.stampPerDevice", "read per device")}
					/>
				) : (
					<FreshnessStamp {...stamp} />
				)
			}
			foot={
				<span>
					{local
						? t(
								"app.spending.footLocal",
								"Local-only apps can't get access to cloud files: their data isn't on the hub.",
							)
						: t(
								"app.spending.foot",
								"A lease lets an instance get cloud credentials for 10 minutes at a time. It isn't proof the instance is healthy. Spending limits are per service on one device.",
							)}
				</span>
			}
		>
			{approved.length ? (
				<>
					<ul className="m-0 list-none p-0">
						{approvedRows.shown.map((entry) =>
							entry.cloud.state === "approved" ? (
								<ApprovalRow
									key={`${entry.row.deviceId}/${entry.row.serviceId}`}
									approval={entry.cloud.approval}
									row={entry.row}
								/>
							) : null,
						)}
					</ul>
					<MoreLine
						shown={approvedRows.shown.length}
						total={approved.length}
						onShow={approvedRows.showAll}
					/>
				</>
			) : local || !hidden.length ? (
				<div className="p-3">
					{unknown.length && !interim ? (
						<StateView
							kind={data.placements.error ? "error" : "notloaded"}
							title={t(
								"app.spending.notLoaded",
								"Cloud access isn't loaded yet",
							)}
						/>
					) : (
						<StateView
							kind="empty"
							icon={local ? Sparkles : Cloud}
							title={
								local
									? t("app.spending.emptyLocalTitle", "No model access")
									: t("app.spending.emptyTitle", "No cloud access you can see")
							}
							text={
								local
									? t(
											"app.spending.emptyLocalText",
											"{{app}}'s services use only what's on the device. You can approve hosted models for a service when you deploy or update it.",
											{ app: view.app.name },
										)
									: t(
											"app.spending.emptyText",
											"None of {{app}}'s services has cloud access that you approved or pay for.",
											{ app: view.app.name },
										)
							}
						/>
					)}
				</div>
			) : null}
			{hiddenRows.shown.map((entry) => (
				<p
					key={`${entry.row.deviceId}/${entry.row.serviceId}`}
					data-approval-hidden={`${entry.row.deviceId}/${entry.row.serviceId}`}
					className="flex items-start gap-1.5 border-t border-hairline px-4 py-2.5 text-ui text-ink-2 first:border-t-0"
				>
					<EyeOff
						aria-hidden
						className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
					/>
					<span>
						<Trans
							t={t}
							i18nKey="app.spending.hidden"
							defaults="<1/> on <2/>: approved by someone else; only they and the device owner see the details."
							components={{
								1: <span className="font-mono">{entry.row.serviceId}</span>,
								2: (
									<span className="font-mono">
										{deviceName(entry.row.deviceId)}
									</span>
								),
							}}
						/>
					</span>
				</p>
			))}
			<MoreLine
				shown={hiddenRows.shown.length}
				total={hidden.length}
				onShow={hiddenRows.showAll}
				hidden
			/>
			{interim ? (
				<p className="border-t border-hairline px-4 py-2 text-xs text-muted-foreground">
					{t(
						"app.spending.interim",
						"This hub has no per-app list of cloud access yet: approvals are read device by device, for the devices you own.",
					)}
				</p>
			) : null}
		</Block>
	);
}
