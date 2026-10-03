"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	CircleCheck,
	Cloud,
	CloudOff,
	Copy,
	Euro,
	RefreshCw,
	Sparkles,
	Timer,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	ActionResults,
	RevokeApprovalButton,
	RevokeLimitButton,
} from "../cloud/action-controls";
import { ApprovalForm, LimitForm } from "../cloud/approval-form";
import { type CloudApproval, currentApproval } from "../cloud/cloud-model";
import {
	AppRef,
	ApprovalChip,
	CloudPerson,
	EndRow,
	ModelList,
	Payer,
	RouteButton,
	SideFact,
	SideFacts,
} from "../cloud/cloud-parts";
import { LeasesTable, leaseNote } from "../cloud/leases-table";
import { SpendByInstance, SpendMeterRow } from "../cloud/spend-meter-row";
import {
	type DeviceApprovals,
	type ServiceBinding,
	useAppNames,
	useAppRole,
	useDeviceApprovals,
	usePausedWrites,
	useServiceBinding,
} from "../cloud/use-cloud";
import { type CloudActions, useCloudActions } from "../cloud/use-cloud-actions";
import { enumExplain, enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { RequestedActual } from "../primitives/requested-actual";
import { StateView } from "../primitives/state-view";
import { useCopy } from "../primitives/use-copy";
import type { ServiceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy, useServiceView } from "../workspace";

type OpenForm = "approve" | "limit" | null;

interface TabContext {
	deviceId: string;
	deviceLabel: string;
	serviceId: string;
	device: DeviceViewModel | undefined;
	service: ServiceView | undefined;
	approvals: DeviceApprovals;
	approval: CloudApproval | undefined;
	binding: ServiceBinding;
	actions: CloudActions;
	/** Offline copies: "model access" wording, never project files. */
	modelOnly: boolean;
	appId: string | null;
	projectId: string | undefined;
	appName: string;
	isAppOwner: boolean | undefined;
	form: OpenForm;
	setForm(form: OpenForm): void;
}

function Stamp({ approvals }: Readonly<{ approvals: DeviceApprovals }>) {
	return <FreshnessStamp {...stampOf(approvals.freshness)} />;
}

function ApproveForm({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { service, approval, actions } = ctx;
	if (ctx.form !== "approve" || !ctx.projectId) return null;
	return (
		<ApprovalForm
			subject={{
				deviceId: ctx.deviceId,
				deviceName: ctx.deviceLabel,
				serviceId: ctx.serviceId,
				appId: ctx.appId,
				projectId: ctx.projectId,
				appName: ctx.appName,
				modelOnly: ctx.modelOnly,
				serviceMaxInstances: service?.instances.max ?? 1,
				...(ctx.isAppOwner === undefined ? {} : { isAppOwner: ctx.isAppOwner }),
				offline: ctx.device?.presence.kind !== "online",
			}}
			current={approval}
			busy={actions.running === "approve"}
			onCancel={() => ctx.setForm(null)}
			onSubmit={(draft, spending) => {
				void actions.approve(draft, spending).then((done) => {
					if (done) ctx.setForm(null);
				});
			}}
		/>
	);
}

/** No approval at all: the service runs with what is on the device. */
function NoApproval({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const { serviceId, actions, modelOnly, form } = ctx;
	const open = form === "approve";
	const offline = ctx.service?.freshness.age === "lastknown";
	return (
		<Block
			id="svc-cloud-approval"
			icon={modelOnly ? Sparkles : Cloud}
			title={
				modelOnly
					? t("cloud.service.titleModel", "Model access")
					: t("cloud.service.title", "Cloud access")
			}
			stamp={<Stamp approvals={ctx.approvals} />}
		>
			<StateView
				kind="empty"
				icon={CloudOff}
				title={t("cloud.service.localOnly", "Runs with local resources only")}
				text={
					<>
						{modelOnly
							? t(
									"cloud.service.noModelAccess",
									"No model access is approved for {{service}}. It runs with what's on the device and can't call hosted models.",
									{ service: serviceId },
								)
							: t(
									"cloud.service.noAccess",
									"No cloud access is approved for {{service}}. It can't call hosted models or read project files in the cloud.",
									{ service: serviceId },
								)}
						{offline
							? ` ${t(
									"cloud.service.bindWaits",
									"Naming a new approval in its settings waits until {{device}} is back online.",
									{ device: ctx.deviceLabel },
								)}`
							: null}
					</>
				}
				actions={
					<GatedAction gate={actions.approveGate}>
						<DvButton
							variant={open || actions.approveGate ? "default" : "primary"}
							icon={modelOnly ? Sparkles : Cloud}
							data-act="approve-open"
							aria-expanded={open}
							onClick={() => ctx.setForm(open ? null : "approve")}
						>
							{modelOnly
								? t("cloud.service.approveModel", "Approve model access…")
								: t("cloud.service.approve", "Approve cloud access…")}
						</DvButton>
					</GatedAction>
				}
			/>
			<ApproveForm ctx={ctx} />
			<ActionResults actions={actions} />
			{modelOnly && !open ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"cloud.fields.localOnly",
						"Local-only apps can't get access to cloud files.",
					)}
				</p>
			) : null}
		</Block>
	);
}

/** Which approval the service's settings name, next to the one shown here. */
function BindingRow({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const { binding, approval, serviceId, deviceId } = ctx;
	if (!approval) return null;
	const active = approval.state === "active";
	const configure = (
		<RouteButton
			size="xs"
			route={{ screen: "service", deviceId, serviceId, tab: "configuration" }}
			data-act="open-configuration"
		>
			{t("cloud.binding.open", "Open Configuration")}
		</RouteButton>
	);
	if (binding.state === "loading" || binding.state === "unread")
		return (
			<span data-binding={binding.state} className="text-muted-foreground">
				{binding.state === "loading"
					? t("cloud.binding.reading", "Reading the service's settings…")
					: binding.refused
						? t(
								"cloud.binding.refused",
								"Needs Deploy & configure on this service to read which approval its settings name.",
							)
						: t(
								"cloud.binding.unread",
								"Read from the service's settings when you're connected live.",
							)}
			</span>
		);
	if (binding.state === "unbound")
		return (
			<div data-binding="unbound" className="flex min-w-0 flex-col gap-1.5">
				<RequestedActual
					versions
					label={t("cloud.binding.label", "Approval the service uses")}
					requested={t("cloud.binding.none", "Service uses no approval")}
					actual={t("cloud.binding.waiting", "this approval is waiting")}
					tone="warning"
					icon={TriangleAlert}
				/>
				<p className="text-xs text-muted-foreground">
					{t(
						"cloud.binding.unboundHint",
						"Not bound yet: {{service}} runs with local resources only until its settings name this approval. Copy the approval reference below into them.",
						{ service: serviceId },
					)}
				</p>
				{configure}
			</div>
		);
	const same = binding.grantId === approval.grantId;
	return (
		<div
			data-binding={same ? "bound" : "other"}
			className="flex min-w-0 flex-col gap-1.5"
		>
			<RequestedActual
				versions
				label={t("cloud.binding.label", "Approval the service uses")}
				requested={
					same
						? t("cloud.binding.this", "Service uses this approval")
						: t("cloud.binding.other", "Service uses another approval")
				}
				actual={
					same
						? active
							? t("cloud.binding.current", "current")
							: enumLabel(t, "approvalStatus", approval.state)
						: t("cloud.binding.notThis", "not this one")
				}
				tone={same ? (active ? "good" : "critical") : "warning"}
				icon={same && active ? CircleCheck : TriangleAlert}
			/>
			{same ? null : (
				<>
					<p className="text-xs text-muted-foreground">
						{t(
							"cloud.binding.otherHint",
							"Its settings name an earlier approval, so it can't use this one yet. Copy the approval reference below into them.",
						)}
					</p>
					{configure}
				</>
			)}
		</div>
	);
}

/** Advanced: what a service's settings must name to use this approval (replaces the old file import). */
function ApprovalReference({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const { approval } = ctx;
	if (!approval?.details) return null;
	const limit =
		approval.limit?.state === "active" && approval.limit.version !== undefined
			? approval.limit
			: undefined;
	// Wire text for a settings file: never translated.
	const binding = JSON.stringify(
		{
			resource_grant: {
				grant_id: approval.grantId,
				authz_version: approval.details.version,
				...(limit
					? {
							billing_grant_id: limit.id,
							billing_authz_version: limit.version,
						}
					: {}),
			},
		},
		null,
		2,
	);
	return (
		<details data-approval-reference="" className="text-ui">
			<summary className="cursor-pointer text-xs text-muted-foreground">
				{t("cloud.reference.summary", "Advanced: approval reference")}
			</summary>
			<div className="mt-2 flex min-w-0 flex-col gap-2">
				<KeyValueList>
					<KvRow label={t("cloud.reference.id", "Approval ID")}>
						<IdRef
							id={approval.grantId}
							copyLabel={t("cloud.reference.copyId", "Copy approval ID")}
						/>
					</KvRow>
					<KvRow label={t("cloud.reference.version", "Version")}>
						<span className="tabular-nums">{approval.details.version}</span>
					</KvRow>
					{limit ? (
						<KvRow label={t("cloud.reference.limitId", "Spending limit ID")}>
							<IdRef
								id={limit.id}
								copyLabel={t(
									"cloud.reference.copyLimitId",
									"Copy spending limit ID",
								)}
							/>
						</KvRow>
					) : null}
				</KeyValueList>
				<pre className="m-0 max-w-full rounded-lg border border-hairline bg-surface-sunken p-2 font-mono text-xs whitespace-pre-wrap wrap-anywhere">
					{binding}
				</pre>
				<div className="flex flex-wrap items-center gap-2">
					<DvButton
						size="xs"
						icon={copied ? Check : Copy}
						data-act="copy-binding"
						onClick={() => void copy(binding)}
					>
						{copied
							? t("cloud.reference.copied", "Copied")
							: t("cloud.reference.copy", "Copy reference")}
					</DvButton>
					<span className="text-xs text-muted-foreground">
						{t(
							"cloud.reference.hint",
							"A service uses an approval once its settings contain this reference (Configuration › Edit as JSON, or the command line).",
						)}
					</span>
				</div>
			</div>
		</details>
	);
}

function ApprovalFacts({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { approval, service, modelOnly } = ctx;
	if (!approval) return null;
	const details = approval.details;
	const serviceMax = service?.instances.max;
	return (
		<KeyValueList>
			<KvRow label={t("cloud.kv.status", "Status")}>
				<ApprovalChip state={approval.state} />
				{details?.approvedAt ? (
					<span className="ml-2 text-muted-foreground">
						{t("cloud.kv.approvedAt", "approved {{date}}", {
							date: time.at(details.approvedAt),
						})}
					</span>
				) : null}
			</KvRow>
			{details?.approvedBy ? (
				<KvRow label={t("cloud.kv.approvedBy", "Approved by")}>
					<CloudPerson userId={details.approvedBy} />
				</KvRow>
			) : null}
			<KvRow label={t("cloud.kv.app", "App")}>
				<AppRef
					appId={approval.appId}
					{...(ctx.projectId ? { projectId: ctx.projectId } : {})}
				/>
			</KvRow>
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
							className="mt-0.5 flex items-start gap-1 text-xs text-warning"
						>
							<TriangleAlert aria-hidden className="mt-px size-3.5 shrink-0" />
							{t(
								"cloud.kv.writeBlocked",
								"Project storage is full, so the service can only read right now. Free up storage to let it write again.",
							)}
						</span>
					) : approval.files === "read_write" ? (
						<span className="mt-0.5 block text-xs text-muted-foreground">
							{enumExplain(t, "onlineAccess", "read_write")}
						</span>
					) : null}
				</KvRow>
			)}
			{details ? (
				<KvRow label={t("cloud.kv.instances", "Instances allowed")}>
					<span className="tabular-nums">{details.maxInstances}</span>
					{serviceMax === undefined ? null : details.maxInstances >=
						serviceMax ? (
						<span className="ml-2 inline-flex items-center gap-1 text-good">
							<CircleCheck aria-hidden className="size-3.5" />
							{t(
								"cloud.kv.instancesOk",
								"at least this service's instances ({{count, number}})",
								{ count: serviceMax },
							)}
						</span>
					) : (
						<span className="ml-2 inline-flex items-center gap-1 text-warning">
							<TriangleAlert aria-hidden className="size-3.5" />
							{t(
								"cloud.kv.instancesLow",
								"below this service's instances ({{count, number}})",
								{ count: serviceMax },
							)}
						</span>
					)}
				</KvRow>
			) : null}
			<EndRow approval={approval} deviceName={ctx.deviceLabel} />
			<KvRow label={t("cloud.kv.binding", "Used by this service")}>
				<BindingRow ctx={ctx} />
			</KvRow>
		</KeyValueList>
	);
}

/** Why the service gets no cloud credentials any more, and what brings them back. */
function StoppedBanner({
	ctx,
	action,
}: Readonly<{ ctx: TabContext; action: ReactNode }>) {
	const { t } = useTranslation("devices");
	const { approval, serviceId, deviceId, modelOnly } = ctx;
	if (!approval || approval.state === "active") return null;
	const said = { service: serviceId, device: ctx.deviceLabel };
	// An approval that ended early comes back by itself once what ended it does.
	const cause = approval.revocable ? approval.effective?.limit : undefined;
	const remedies = {
		access_rules: t(
			"cloud.service.resumeRules",
			"It resumes when the access rules of {{device}} are renewed.",
			said,
		),
		sharing_grant: t(
			"cloud.service.resumeSharing",
			"It resumes when its approver may deploy on {{device}} again. You can also replace it with an approval of your own.",
			said,
		),
		approval: modelOnly
			? t(
					"cloud.service.approveAgainModel",
					"Approve model access again; the service's settings then need to name the new approval.",
				)
			: t(
					"cloud.service.approveAgain",
					"Approve cloud access again; the service's settings then need to name the new approval.",
				),
	};
	return (
		<Banner
			tone="critical"
			title={
				approval.state === "revoked"
					? t(
							"cloud.service.revokedTitle",
							"{{service}} is bound to a revoked approval.",
							said,
						)
					: t(
							"cloud.service.endedTitle",
							"The approval of {{service}} has ended.",
							said,
						)
			}
			actions={
				<>
					{action}
					{cause === "access_rules" ? (
						<RouteButton
							size="sm"
							route={{ screen: "device", deviceId, tab: "access" }}
							data-act="open-access"
						>
							{t("cloud.service.openAccess", "Open access rules")}
						</RouteButton>
					) : null}
				</>
			}
		>
			{modelOnly
				? t(
						"cloud.service.stoppedModel",
						"Its instances can't get cloud credentials, so model calls fail.",
					)
				: t(
						"cloud.service.stopped",
						"Its instances can't get cloud credentials: model calls and project-file access fail. Buffered writes pause and are kept.",
					)}{" "}
			{remedies[cause ?? "approval"]}
		</Banner>
	);
}

function ApprovalBlock({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const { approval, actions, modelOnly, serviceId, deviceId, service } = ctx;
	const paused = usePausedWrites(deviceId, serviceId, service);
	if (!approval) return null;
	const active = approval.state === "active";
	const open = ctx.form === "approve";
	const approveAgain = (
		<GatedAction gate={actions.approveGate}>
			<DvButton
				size="sm"
				icon={modelOnly ? Sparkles : Cloud}
				data-act="approve-open"
				aria-expanded={open}
				onClick={() => ctx.setForm(open ? null : "approve")}
			>
				{modelOnly
					? t("cloud.service.approveModel", "Approve model access…")
					: t("cloud.service.approve", "Approve cloud access…")}
			</DvButton>
		</GatedAction>
	);
	return (
		<Block
			id="svc-cloud-approval"
			icon={modelOnly ? Sparkles : Cloud}
			title={t("cloud.service.titleApproval", "Approval")}
			stamp={<Stamp approvals={ctx.approvals} />}
		>
			<StoppedBanner ctx={ctx} action={approveAgain} />
			{paused && active ? (
				<Banner
					tone="warning"
					title={
						paused.exact
							? t("cloud.service.pausedTitle", {
									count: paused.count,
									defaultValue_one:
										"{{count, number}} buffered change is paused because cloud access changed.",
									defaultValue_other:
										"{{count, number}} buffered changes are paused because cloud access changed.",
								})
							: t(
									"cloud.service.pausedTitleSome",
									"Buffered changes are paused because cloud access changed.",
								)
					}
					actions={
						<RouteButton
							size="sm"
							route={{ screen: "service", deviceId, serviceId, tab: "offline" }}
							data-act="open-buffering"
						>
							{t("cloud.service.openBuffering", "Open Write buffering")}
						</RouteButton>
					}
				>
					{t(
						"cloud.service.pausedText",
						"They were buffered under an earlier approval and are kept on the device. Move them to the current approval on the Write buffering tab.",
					)}
				</Banner>
			) : null}
			<ApprovalFacts ctx={ctx} />
			<ApprovalReference ctx={ctx} />
			{approval.revocable ? (
				<div className="flex flex-wrap items-start gap-2 border-t border-hairline pt-3">
					{active ? (
						<GatedAction gate={actions.approveGate}>
							<DvButton
								icon={RefreshCw}
								data-act="approve-open"
								aria-expanded={open}
								onClick={() => ctx.setForm(open ? null : "approve")}
							>
								{t("cloud.service.replaceApproval", "Replace approval…")}
							</DvButton>
						</GatedAction>
					) : null}
					<RevokeApprovalButton actions={actions} />
				</div>
			) : null}
			<ApproveForm ctx={ctx} />
			<ActionResults actions={actions} about="approval" />
		</Block>
	);
}

function LeasesBlock({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const { approval, binding } = ctx;
	const leases = approval?.leases;
	// Not read yet counts as named: only a known mismatch explains missing leases.
	const named =
		binding.state === "bound"
			? binding.grantId === approval?.grantId
			: binding.state !== "unbound";
	const checks = leases?.some(
		(lease) => lease.purpose === "rollout_validation",
	);
	return (
		<Block
			id="svc-cloud-leases"
			icon={Timer}
			title={t("cloud.leases.title", "Cloud leases")}
			{...(leases ? { count: leases.length } : {})}
			stamp={<Stamp approvals={ctx.approvals} />}
			flush={!!leases?.length}
			foot={
				<span>
					{leaseNote(t)}
					{checks
						? ` ${t(
								"cloud.leases.checksNote",
								"An update check doesn't count toward the instances allowed.",
							)}`
						: null}
				</span>
			}
		>
			{leases?.length ? (
				<LeasesTable leases={leases} />
			) : (
				<p className="text-ui text-muted-foreground">
					{leases === undefined
						? t(
								"cloud.leases.hidden",
								"Only the approver and the device owner see which instances hold a lease.",
							)
						: named
							? t(
									"cloud.leases.none",
									"No instance holds a lease right now. Instances request one on their first cloud call.",
								)
							: t(
									"cloud.leases.unbound",
									"No leases: instances can get cloud credentials only once the service's settings name this approval.",
								)}
				</p>
			)}
		</Block>
	);
}

function SpendingBlock({ ctx }: Readonly<{ ctx: TabContext }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { approval, actions, deviceId, serviceId } = ctx;
	if (!approval) return null;
	const limit = approval.limit;
	const live = limit?.state === "active";
	const open = ctx.form === "limit";
	const hasModels = (approval.details?.models.length ?? 1) > 0;
	const opener = (label: string) => (
		<GatedAction gate={actions.limitGate}>
			<DvButton
				icon={Euro}
				data-act="limit-open"
				aria-expanded={open}
				onClick={() => ctx.setForm(open ? null : "limit")}
			>
				{label}
			</DvButton>
		</GatedAction>
	);
	let body: ReactNode;
	if (limit && live)
		body = (
			<>
				<SpendMeterRow limit={limit} end={false} />
				<SideFacts>
					<SideFact label={t("cloud.spend.paidBy", "Paid by")}>
						<Payer approval={approval} />
					</SideFact>
					<SideFact label={t("cloud.spend.ends", "Ends")}>
						{t("cloud.spend.endsValue", "{{date}} · not recurring", {
							date: time.at(limit.expiresAt),
						})}
					</SideFact>
					<SideFact label={t("cloud.spend.perInstance", "Per-instance spend")}>
						<SpendByInstance deviceId={deviceId} limit={limit} />
					</SideFact>
				</SideFacts>
				<div className="flex flex-wrap items-start gap-2">
					{opener(t("cloud.spend.replaceLimit", "Replace spending limit…"))}
					<RevokeLimitButton actions={actions} />
				</div>
				<p className="text-xs text-muted-foreground">
					{t(
						"cloud.spend.replaceHint",
						"Raising the limit means replacing it: the new limit starts from zero used and the old one closes.",
					)}
				</p>
			</>
		);
	else if (!hasModels)
		body = (
			<p className="text-ui text-muted-foreground">
				{t(
					"cloud.spend.noModels",
					"This approval has no models, so nothing is charged and no spending limit is needed.",
				)}
			</p>
		);
	else
		body = (
			<>
				{limit ? <SpendMeterRow limit={limit} /> : null}
				<p data-no-limit="" className="text-ui">
					{t(
						"cloud.spend.none",
						"No spending limit: hosted models refuse {{service}}'s model calls until someone sets one.",
						{ service: serviceId },
					)}
				</p>
				<div className="flex flex-wrap items-start gap-2">
					{opener(t("cloud.spend.add", "Add spending limit…"))}
				</div>
			</>
		);
	return (
		<Block
			id="svc-cloud-spending"
			icon={Euro}
			title={t("cloud.spend.title", "Spending limit")}
			stamp={<Stamp approvals={ctx.approvals} />}
		>
			{body}
			{open ? (
				<LimitForm
					deviceId={deviceId}
					approval={approval}
					replace={live ? limit : undefined}
					busy={actions.running === "addLimit"}
					onCancel={() => ctx.setForm(null)}
					onSubmit={(spending) => {
						void actions.addLimit(spending).then((done) => {
							if (done) ctx.setForm(null);
						});
					}}
				/>
			) : null}
			<ActionResults actions={actions} about="limit" />
		</Block>
	);
}

/** SPEC §5.3 Cloud access: the approval a service runs on, its spending limit and its leases. */
export function ServiceCloudTab({
	deviceId,
	serviceId,
}: Readonly<ServiceTabProps>) {
	const { t } = useTranslation("devices");
	const { service, device } = useServiceView(deviceId, serviceId);
	const approvals = useDeviceApprovals(deviceId);
	const names = useAppNames();
	const [form, setForm] = useState<OpenForm>(null);

	const approval = currentApproval(approvals.rows, serviceId);
	const projectId = service?.projectId ?? approval?.details?.projectId;
	const source =
		service?.source ??
		(approval ? (approval.appId ? "online" : "offline") : null);
	const modelOnly = source === "offline";
	const appId =
		source === "online" ? (projectId ?? approval?.appId ?? null) : null;
	const deploymentId = service?.deploymentId ?? approval?.details?.deploymentId;
	const deviceLabel = device ? deviceName(device.row) : deviceId.slice(0, 8);
	const appName =
		(projectId ? names(projectId) : undefined) ??
		t("cloud.fields.thisApp", "this app");
	const role = useAppRole(appId);
	const binding = useServiceBinding(deviceId, serviceId, projectId);
	const actions = useCloudActions({
		deviceId,
		deviceName: deviceLabel,
		serviceId,
		approval,
		identity:
			projectId && deploymentId && source
				? { projectId, deploymentId, appId }
				: undefined,
		modelOnly,
		...(role.projectRole ? { projectRole: role.projectRole } : {}),
		sub: t("cloud.service.context", "{{app}} · on {{device}}", {
			app: appName,
			device: deviceLabel,
		}),
	});

	const ctx: TabContext = {
		deviceId,
		deviceLabel,
		serviceId,
		device,
		service,
		approvals,
		approval,
		binding,
		actions,
		modelOnly,
		appId,
		projectId,
		appName,
		isAppOwner: role.known ? role.owner : undefined,
		form,
		setForm,
	};

	if (!approvals.loaded)
		return (
			<div data-service-cloud="" className="flex min-w-0 flex-col gap-4">
				{approvals.error ? (
					<StateView
						kind="error"
						title={t(
							"cloud.service.loadFailed",
							"Couldn't read the cloud access of this device",
						)}
						text={t(
							"cloud.service.loadFailedText",
							"{{why}} Nothing is known about {{service}}'s cloud access until this can be read.",
							{
								why: hubErrorCopy(t, approvals.error.code),
								service: serviceId,
							},
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
						title={t("cloud.service.loading", "Reading cloud access…")}
					/>
				)}
			</div>
		);

	if (!approval)
		return (
			<div data-service-cloud="none" className="flex min-w-0 flex-col gap-4">
				<NoApproval ctx={ctx} />
			</div>
		);

	// Two columns once the tab itself is wide enough for the leases table next to the limit.
	return (
		<div
			data-service-cloud={approval.state}
			className="@container/cloud min-w-0"
		>
			<div className="grid min-w-0 items-start gap-4 @min-[960px]/cloud:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
				<ApprovalBlock ctx={ctx} />
				<div className="min-w-0 @min-[960px]/cloud:row-span-2">
					<SpendingBlock ctx={ctx} />
				</div>
				<LeasesBlock ctx={ctx} />
			</div>
		</div>
	);
}
