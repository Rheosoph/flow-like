"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	BadgeCheck,
	BadgeEuro,
	ChevronRight,
	Cloud,
	Copy,
	Database,
	Info,
	Users,
} from "lucide-react";
import { type FocusEvent, Fragment, useEffect, useMemo } from "react";
import { useAppPermissions } from "../../../../../hooks/use-app-permissions";
import type { OfflineWritesConfig } from "../../../../../lib/device-management/deployment";
import type {
	ApprovalDraft,
	DeployPlan,
	DeployTargetDraft,
	PlanTarget,
	PlanTargetService,
	SpendingDraft,
} from "../../../../../lib/device-management/model/deploy-plan";
import { evaluateGate } from "../../../../../lib/device-management/model/gates";
import type {
	AppDevicePlacements,
	GateResult,
} from "../../../../../lib/device-management/model/types";
import {
	eurosToMicros,
	formatEuroMicros,
} from "../../../../../lib/device-resources";
import { RolePermissions } from "../../../../../lib/permission/role-permission";
import {
	CloudApprovalFields,
	SpendingLimitFields,
} from "../../cloud/approval-fields";
import { gateCopy } from "../../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { Checklist, type ChecklistItem } from "../../primitives/checklist";
import { DvButton } from "../../primitives/dv-button";
import { Field, InputWithUnit } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GateNotice } from "../../primitives/gate-notice";
import { type SegmentOption, Segmented } from "../../primitives/segmented";
import { useCopy } from "../../primitives/use-copy";
import { WizardStepHeader } from "../../primitives/wizard";
import { stampOf } from "../../shell/attention-popover";
import {
	buildGateContext,
	useAppPlacements,
	useAttentionState,
	useDeviceRows,
} from "../../workspace";
import type { DeployStepProps } from "../step-props";
import { deployRunExtras, deployWhat } from "../use-deploy-run";
import { WriteBufferingFields } from "./write-buffering-fields";

/* Step 6 online · Access & cost (APP §3.10): every new service on every device gets its own approval. */

const DEFAULT_LIMIT_MICROS = 10_000_000;

interface NewService {
	target: PlanTarget;
	service: PlanTargetService;
}

export const newServices = (plan: DeployPlan): NewService[] =>
	plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.kind === "new" && service.events.length > 0)
			.map((service) => ({ target, service })),
	);

export const keptServices = (plan: DeployPlan): NewService[] =>
	plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.kind !== "new")
			.map((service) => ({ target, service })),
	);

function HubStamp() {
	const { freshness } = useDeviceRows();
	return <FreshnessStamp {...stampOf(freshness)} />;
}

function ChoiceStamp() {
	const { t } = useTranslation("devices");
	return (
		<FreshnessStamp
			source="local"
			age="current"
			text={t("deployShip.access.yourChoice", "your choice")}
		/>
	);
}

/* Who can approve. */

interface Approver {
	/** False while the role can't be read: nothing is claimed either way. */
	known: boolean;
	loading: boolean;
	owner: boolean;
	canApprove: boolean;
	roleName?: string;
	devices: { target: PlanTarget; gate: GateResult; mine: boolean }[];
	/** The first reason a new approval can't be created by this account. */
	blocked: boolean;
}

function useApprover(plan: DeployPlan, fresh: readonly NewService[]): Approver {
	const appId = plan.app?.id ?? "";
	const permissions = useAppPermissions(appId);
	const sources = useAttentionState();
	const { workspace, input, tokenScopeAll } = sources;
	const ids = [...new Set(fresh.map((row) => row.target.deviceId))].join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `ids` stands for the target list
	const devices = useMemo(
		() =>
			plan.targets
				.filter((target) => ids.split("|").includes(target.deviceId))
				.map((target) => ({
					target,
					mine: input.devices.some(
						(row) =>
							row.device_id === target.deviceId && row.owner_id === input.me,
					),
					gate: evaluateGate(
						"cloud_access_create",
						buildGateContext(
							{ workspace, input, tokenScopeAll },
							target.deviceId,
							{ projectId: appId },
						),
					),
				})),
		[workspace, input, tokenScopeAll, ids, appId],
	);
	const canApprove =
		permissions.known &&
		permissions.isOwner &&
		permissions.canStrict(RolePermissions.ExecuteBoards);
	return {
		known: permissions.known,
		loading: permissions.isLoading,
		// `isOwner` also holds for an Admin; project files need the Owner permission itself.
		owner:
			permissions.known &&
			permissions.permissions.contains(RolePermissions.Owner),
		canApprove,
		...(permissions.roleName ? { roleName: permissions.roleName } : {}),
		devices,
		blocked: permissions.known && !canApprove,
	};
}

function ownerCheck(
	t: DevicesT,
	app: string,
	approver: Approver,
	wantsFiles: boolean,
): ChecklistItem {
	if (!wantsFiles)
		return {
			id: "owner",
			state: "pass",
			source: "hub",
			label: t(
				"devices:deployShip.access.ownerNotNeeded",
				"Project files aren't requested, so ownership isn't needed",
			),
		};
	if (!approver.known)
		return {
			id: "owner",
			state: "pending",
			source: "hub",
			label: t(
				"devices:deployShip.access.ownerUnknown",
				"Whether you own {{app}} isn't known yet (needed for project files)",
				{ app },
			),
		};
	return approver.owner
		? {
				id: "owner",
				state: "pass",
				source: "hub",
				label: t(
					"devices:deployShip.access.ownerYes",
					"You own {{app}} (needed for project files)",
					{ app },
				),
			}
		: {
				id: "owner",
				state: "fail",
				source: "hub",
				label: t(
					"devices:deployShip.access.ownerNo",
					"Only the owner of {{app}} can allow project files",
					{ app },
				),
			};
}

function roleCheck(t: DevicesT, approver: Approver): ChecklistItem {
	const label = t(
		"devices:deployShip.access.role",
		"Admin or Owner with Execute boards on the app",
	);
	if (!approver.known)
		return { id: "role", state: "pending", source: "hub", label };
	if (approver.canApprove)
		return { id: "role", state: "pass", source: "hub", label };
	return {
		id: "role",
		state: "fail",
		source: "hub",
		label,
		note: approver.roleName
			? t("devices:deployShip.access.roleIs", "Your role: {{role}}", {
					role: approver.roleName,
				})
			: undefined,
	};
}

function deviceCheck(
	t: DevicesT,
	time: AreaTime,
	approver: Approver,
): ChecklistItem {
	const failing = approver.devices.find((row) => !row.gate.ok);
	const label = (
		<>
			{t(
				"devices:deployShip.access.devicesLead",
				"Deploy & configure on each device:",
			)}
			{approver.devices.map((row, index) => (
				<Fragment key={row.target.deviceId}>
					{index ? " · " : " "}
					<span className="font-mono">{row.target.name}</span>{" "}
					{row.mine
						? t("devices:deployShip.access.yours", "(yours)")
						: t("devices:deployShip.access.shared", "(shared with you)")}
				</Fragment>
			))}
		</>
	);
	if (!failing || failing.gate.ok)
		return { id: "devices", state: "pass", source: "hub", label };
	return {
		id: "devices",
		state: "fail",
		source: "hub",
		label,
		note: `${failing.target.name}: ${gateCopy(t, failing.gate, time).inline}`,
	};
}

function ApproverBlock({
	plan,
	approver,
}: Readonly<{ plan: DeployPlan; approver: Approver }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const app = plan.app?.name ?? "";
	const wantsFiles = plan.draft.approval.files !== "none";
	return (
		<Block
			id="dp-who"
			icon={Users}
			title={t("deployShip.access.who", "Who can approve")}
			stamp={<HubStamp />}
		>
			<Checklist
				label={t("deployShip.access.who", "Who can approve")}
				items={[
					ownerCheck(t, app, approver, wantsFiles),
					roleCheck(t, approver),
					deviceCheck(t, time, approver),
				]}
			/>
		</Block>
	);
}

function ApproverGate({
	plan,
	approver,
	fresh,
}: Readonly<{
	plan: DeployPlan;
	approver: Approver;
	fresh: readonly NewService[];
}>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const app = plan.app?.name ?? "";
	const request = t(
		"deployShip.access.request",
		"Hi, could you approve cloud access for {{app}} on {{devices}}? I'm deploying {{what}} there. Thanks!",
		{
			app,
			what: deployWhat(plan),
			devices: [...new Set(fresh.map((row) => row.target.name))].join(", "),
		},
	);
	return (
		<GateNotice
			kind="role"
			title={t(
				"deployShip.access.gateTitle",
				"You can't approve cloud access for this app.",
			)}
			text={t(
				"deployShip.access.gateText",
				"New online services need cloud access from an Admin or Owner of {{app}} with Execute boards. Updates of existing services keep their approval.",
				{ app },
			)}
			have={
				approver.roleName
					? t("deployShip.access.gateHave", "Your role on {{app}}: {{role}}", {
							app,
							role: approver.roleName,
						})
					: undefined
			}
			actions={
				<DvButton size="sm" icon={Copy} onClick={() => void copy(request)}>
					{copied
						? t("deployShip.access.requestCopied", "Copied")
						: t(
								"deployShip.access.copyRequest",
								"Copy a request for the owner",
							)}
				</DvButton>
			}
		/>
	);
}

/* Cloud access and spending. */

export const defaultSpending = (approval: ApprovalDraft): SpendingDraft => ({
	limitMicros: DEFAULT_LIMIT_MICROS,
	expiresAt: approval.expiresAt,
	consent: false,
});

interface FormProps
	extends Pick<DeployStepProps, "draft" | "update" | "prepared"> {
	plan: DeployPlan;
	fresh: readonly NewService[];
	blocked: boolean;
	/** The hub says the viewer isn't the app's owner: project files can't be allowed. */
	notOwner?: boolean;
}

/** "Creates 2 approvals, one for visitor-checkin on each device, …" and why there is no shared budget. */
export function ApprovalsLine({
	plan,
	fresh,
	modelOnly = false,
}: Readonly<{
	plan: DeployPlan;
	fresh: readonly NewService[];
	modelOnly?: boolean;
}>) {
	const { t } = useTranslation("devices");
	if (!fresh.length) return null;
	const services = [...new Set(fresh.map((row) => row.service.serviceId))];
	const values = { count: fresh.length, services: services.join(", ") };
	const components = {
		1: <b className="font-semibold" />,
		2: <span className="font-mono" />,
	};
	const oneModels = {
		defaultValue_one:
			"Creates <1>{{count, number}} approval</1> for <2>{{services}}</2> with these models and end date.",
	};
	const oneAll = {
		defaultValue_one:
			"Creates <1>{{count, number}} approval</1> for <2>{{services}}</2> with these files, models and end date.",
	};
	return (
		<>
			<p
				className="flex items-start gap-1.5 text-ui text-ink-2"
				data-approvals=""
			>
				<BadgeCheck aria-hidden className="mt-0.5 size-3.5 shrink-0" />
				<span>
					{modelOnly ? (
						<Trans
							t={t}
							i18nKey="deployShip.access.createsModels"
							defaults="Creates <1>{{count, number}} approvals</1>, one for <2>{{services}}</2> on each device, with the same models and end date."
							tOptions={oneModels}
							values={values}
							components={components}
						/>
					) : (
						<Trans
							t={t}
							i18nKey="deployShip.access.createsAll"
							defaults="Creates <1>{{count, number}} approvals</1>, one for <2>{{services}}</2> on each device, with the same files, models and end date."
							tOptions={oneAll}
							values={values}
							components={components}
						/>
					)}
				</span>
			</p>
			{plan.targets.length > 1 ? (
				<details className="group text-ui">
					<summary className="flex cursor-pointer list-none items-center gap-1.5 [&::-webkit-details-marker]:hidden">
						<ChevronRight
							aria-hidden
							className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90"
						/>
						{t("deployShip.access.oneBudgetAsk", "One budget for all devices?")}
					</summary>
					<p className="mt-1.5 pl-5 text-xs text-muted-foreground">
						{t(
							"deployShip.access.oneBudget",
							"Not yet. Spending limits belong to one approval, so each device gets its own. Review shows the most it can cost in total.",
						)}
					</p>
				</details>
			) : null}
		</>
	);
}

/** What an approval change takes with it: models need a spending limit, and only Read & write can buffer writes. */
function approvalPatch(
	draft: FormProps["draft"],
	approval: ApprovalDraft,
): Partial<FormProps["draft"]> {
	const spending = approval.models.length
		? (draft.spending ?? defaultSpending(approval))
		: null;
	return approval.files === "read_write"
		? { approval, spending }
		: { approval, spending, writes: null };
}

const GATED_REASON = { code: "role_admin_execute" as const };

function CloudBlock(form: Readonly<FormProps>) {
	const { plan, draft, update, fresh, blocked } = form;
	const { t } = useTranslation("devices");
	const first = fresh.length ? fresh[0].target : plan.targets[0];
	const deviceId = first ? first.deviceId : "";
	const appId = plan.app ? plan.app.id : null;
	const foot = t(
		"deployShip.access.cloudFoot",
		"Services get cloud credentials 10 minutes at a time. That isn't proof a service is healthy.",
	);
	function setApproval(approval: ApprovalDraft) {
		update(approvalPatch(draft, approval));
	}
	return (
		<Block
			id="dp-cloud"
			icon={Cloud}
			title={t("deployShip.access.cloud", "Cloud access")}
			stamp={<HubStamp />}
			bodyClassName="flex flex-col gap-3"
			foot={foot}
		>
			<CloudApprovalFields
				deviceId={deviceId}
				appId={appId}
				modelBits={form.prepared?.artifact.bits}
				value={draft.approval}
				onChange={setApproval}
				modelOnly={false}
				disabledReason={blocked ? GATED_REASON : undefined}
				serviceMaxInstances={draft.maxInstances}
				isAppOwner={form.notOwner ? false : undefined}
			/>
			<ApprovalsLine plan={plan} fresh={fresh} />
		</Block>
	);
}

function limitOf(draft: FormProps["draft"], target: DeployTargetDraft): number {
	const own = target.over.spendingLimitMicros;
	if (own !== undefined) return own;
	return draft.spending ? draft.spending.limitMicros : DEFAULT_LIMIT_MICROS;
}

function amountText(micros: number): string {
	return formatEuroMicros(micros).replace("€", "");
}

/** The typed amount in micros; undefined while it isn't a valid amount. */
function parseLimit(text: string): number | undefined {
	try {
		return eurosToMicros(text);
	} catch (_error) {
		return undefined;
	}
}

function withLimit(
	targets: readonly DeployTargetDraft[],
	deviceId: string,
	micros: number,
): DeployTargetDraft[] {
	return targets.map(function limited(target) {
		if (target.deviceId !== deviceId) return target;
		return { ...target, over: { ...target.over, spendingLimitMicros: micros } };
	});
}

interface LimitFieldProps {
	name: string;
	target: DeployTargetDraft;
	value: number;
	onLimit(deviceId: string, text: string): void;
}

function LimitField({
	name,
	target,
	value,
	onLimit,
}: Readonly<LimitFieldProps>) {
	const label = <span className="font-mono">{name}</span>;
	function commit(event: FocusEvent<HTMLInputElement>) {
		onLimit(target.deviceId, event.target.value);
	}
	return (
		<Field id={`dp-limit-${target.deviceId}`} label={label}>
			<InputWithUnit
				unit="EUR"
				numeric
				inputMode="decimal"
				defaultValue={amountText(value)}
				onBlur={commit}
			/>
		</Field>
	);
}

function PerDeviceLimits({ plan, draft, update, fresh }: Readonly<FormProps>) {
	const { t } = useTranslation("devices");
	const ids = new Set(fresh.map((row) => row.target.deviceId));
	const names = new Map(plan.targets.map((row) => [row.deviceId, row.name]));
	const targets = draft.targets.filter((target) => ids.has(target.deviceId));
	function setLimit(deviceId: string, text: string) {
		const micros = parseLimit(text);
		if (micros === undefined) return;
		update({ targets: withLimit(draft.targets, deviceId, micros) });
	}
	return (
		<div className="flex flex-col gap-1.5">
			{targets.map((target) => (
				<LimitField
					key={target.deviceId}
					name={names.get(target.deviceId) ?? target.deviceId}
					target={target}
					value={limitOf(draft, target)}
					onLimit={setLimit}
				/>
			))}
			<p className="text-xs text-muted-foreground">
				{t(
					"deployShip.access.perDeviceHint",
					"Each amount is the most that device's service can spend on models.",
				)}
			</p>
		</div>
	);
}

type LimitMode = "same" | "per";

/** Same on each device drops the devices' own limits; Different per device starts each from the shared one. */
function withLimitMode(
	draft: FormProps["draft"],
	mode: LimitMode,
): DeployTargetDraft[] {
	return draft.targets.map(function moded(target) {
		const { spendingLimitMicros: _own, ...over } = target.over;
		if (mode === "same") return { ...target, over };
		const spendingLimitMicros = limitOf(draft, target);
		return { ...target, over: { ...over, spendingLimitMicros } };
	});
}

export function SpendingBlock(props: Readonly<FormProps>) {
	const { plan, draft, update, fresh } = props;
	const { t } = useTranslation("devices");
	const spending = draft.spending ?? defaultSpending(draft.approval);
	const targets = draft.targets.filter((target) =>
		fresh.some((row) => row.target.deviceId === target.deviceId),
	);
	const perDevice = targets.some(
		(target) => target.over.spendingLimitMicros !== undefined,
	);
	const total = targets.reduce(
		(sum, target) => sum + limitOf(draft, target),
		0,
	);
	const modeLabel = t("deployShip.access.limitMode", "On each device");
	const modes: SegmentOption<LimitMode>[] = [
		{
			value: "same",
			label: t("deployShip.access.same", "Same on each device"),
		},
		{ value: "per", label: t("deployShip.access.per", "Different per device") },
	];
	function setMode(mode: LimitMode) {
		update({ targets: withLimitMode(draft, mode) });
	}
	function setSpending(next: SpendingDraft) {
		update({ spending: next });
	}
	return (
		<Block
			id="dp-spend"
			icon={BadgeEuro}
			title={t("deployShip.access.spending", "Spending limit")}
			stamp={<ChoiceStamp />}
			bodyClassName="flex flex-col gap-3"
		>
			<p className="flex items-start gap-1.5 text-ui text-ink-2">
				<Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
				<span>
					{t(
						"deployShip.access.spendingWhy",
						"An approval with models needs a spending limit. The services can call the models only up to it.",
					)}
				</span>
			</p>
			<SpendingLimitFields
				deviceId={fresh[0]?.target.deviceId ?? ""}
				value={spending}
				approval={draft.approval}
				onChange={setSpending}
				services={targets.length}
			/>
			{targets.length > 1 ? (
				<div className="flex flex-col items-start gap-1.5" data-limit-mode="">
					<span className="text-ui font-medium">{modeLabel}</span>
					<Segmented
						label={modeLabel}
						value={perDevice ? "per" : "same"}
						onChange={setMode}
						options={modes}
					/>
				</div>
			) : null}
			{perDevice ? <PerDeviceLimits {...props} /> : null}
			<SumLine
				each={formatEuroMicros(spending.limitMicros)}
				total={formatEuroMicros(total)}
				devices={targets.length}
				perDevice={perDevice}
			/>
		</Block>
	);
}

/** BG-A2: there is one limit per device, so the page states the most all of them can cost together. */
function SumLine({
	each,
	total,
	devices,
	perDevice,
}: Readonly<{
	each: string;
	total: string;
	devices: number;
	perDevice: boolean;
}>) {
	const { t } = useTranslation("devices");
	const bold = { 1: <b className="font-semibold" /> };
	const tail = t(
		"deployShip.access.sumTail",
		" · paid by you · doesn't renew · ends with the approval",
	);
	return (
		<p
			className="flex items-start gap-1.5 text-ui text-ink-2"
			data-spend-sum=""
		>
			<BadgeEuro aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span>
				{devices <= 1 ? (
					<Trans
						t={t}
						i18nKey="deployShip.access.sumSingle"
						defaults="<1>{{total}}</1> at most"
						values={{ total }}
						components={bold}
					/>
				) : perDevice ? (
					<Trans
						t={t}
						i18nKey="deployShip.access.sumPer"
						defaults="Your limits per device · <1>{{total}}</1> at most in total"
						values={{ total }}
						components={bold}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="deployShip.access.sumSame"
						defaults="<1>{{each}}</1> on each device · <1>{{total}}</1> at most in total"
						values={{ each, total }}
						components={bold}
					/>
				)}
				{tail}
			</span>
		</p>
	);
}

function WritesBlock({
	plan,
	draft,
	update,
	blocked,
	notOwner,
}: Readonly<FormProps>) {
	const { t } = useTranslation("devices");
	const readWrite = draft.approval.files === "read_write";
	const maxInstances = Math.max(
		draft.maxInstances,
		...plan.services.map((service) => service.maxInstances),
	);
	return (
		<Block
			id="dp-writes"
			icon={Database}
			title={t("deployShip.access.writes", "Write buffering")}
			stamp={<ChoiceStamp />}
		>
			{readWrite ? (
				<WriteBufferingFields
					value={draft.writes ?? null}
					maxInstances={maxInstances}
					disabled={blocked}
					onChange={(writes: OfflineWritesConfig | null) => update({ writes })}
				/>
			) : (
				<GateNotice
					kind="policy"
					title={t(
						"deployShip.access.writesGate",
						"Write buffering needs Read & write project files.",
					)}
					text={t(
						"deployShip.access.writesGateText",
						"It keeps accepting changes when the internet drops and sends them when it's back.",
					)}
					actions={
						// Only the app's owner can allow its files; Cloud access above says so.
						<DvButton
							size="sm"
							disabled={blocked || notOwner}
							onClick={() =>
								update({
									approval: { ...draft.approval, files: "read_write" },
								})
							}
						>
							{t("deployShip.access.allowReadWrite", "Allow Read & write")}
						</DvButton>
					}
				/>
			)}
		</Block>
	);
}

/* Services that keep their approval. */

type Placement = AppDevicePlacements["placements"][number];

interface KeptContext {
	t: DevicesT;
	time: AreaTime;
	/** The hub's list of this app's approvals is known (older hubs have none). */
	listed: boolean;
	/** The hub's clock when it listed them, unix seconds; 0 while unknown. */
	hubNow: number;
	modelOnly: boolean;
}

function unlistedLine(
	c: KeptContext,
	params: { service: string; device: string },
): string {
	if (c.listed && c.modelOnly)
		return c.t(
			"devices:deployShip.kept.noModels",
			"{{service}} on {{device}} has no model access; that stays as it is.",
			params,
		);
	return c.modelOnly
		? c.t(
				"devices:deployShip.kept.plainModels",
				"{{service}} on {{device}} keeps its model access and spending limit.",
				params,
			)
		: c.t(
				"devices:deployShip.kept.plain",
				"{{service}} on {{device}} keeps its cloud access and spending limit.",
				params,
			);
}

function endedLine(
	c: KeptContext,
	params: { service: string; device: string; until: string },
): string {
	return c.modelOnly
		? c.t(
				"devices:deployShip.kept.endedModels",
				"{{service}} on {{device}}: its model access ended {{until}} and the update doesn't renew it.",
				params,
			)
		: c.t(
				"devices:deployShip.kept.ended",
				"{{service}} on {{device}}: its cloud access ended {{until}} and the update doesn't renew it.",
				params,
			);
}

function keptLine(
	c: KeptContext,
	row: NewService,
	placement: Placement | undefined,
): string {
	const { t, time } = c;
	const service = row.service.serviceId;
	const device = row.target.name;
	if (!placement) return unlistedLine(c, { service, device });
	const ends = placement.grant.effective_expires_at;
	const until = time.at(ends);
	// At or before the hub's now the value is when the approval really ended.
	if (ends <= c.hubNow) return endedLine(c, { service, device, until });
	const billing = placement.billing;
	return billing
		? t(
				"devices:deployShip.kept.withSpending",
				"{{service}} on {{device}} keeps its cloud access (until {{until}}) and spending limit ({{used}} of {{limit}}).",
				{
					service,
					device,
					until,
					used: formatEuroMicros(billing.used_micros),
					limit: formatEuroMicros(billing.limit_micros),
				},
			)
		: t(
				"devices:deployShip.kept.access",
				"{{service}} on {{device}} keeps its cloud access (until {{until}}).",
				{ service, device, until },
			);
}

export function KeptBlock({
	plan,
	kept,
	modelOnly = false,
}: Readonly<{
	plan: DeployPlan;
	kept: readonly NewService[];
	/** Offline copies: the approval only covers hosted models. */
	modelOnly?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const placements = useAppPlacements(plan.app?.id);
	if (!kept.length) return null;
	const c: KeptContext = {
		t,
		time,
		listed: placements.data !== undefined,
		hubNow: placements.data ? placements.data.server_time : 0,
		modelOnly,
	};
	const find = (row: NewService) =>
		placements.data?.placements.find(
			(placement) =>
				placement.device_id === row.target.deviceId &&
				placement.placement_id === row.service.serviceId,
		);
	return (
		<Block
			id="dp-kept"
			icon={BadgeCheck}
			title={t("deployShip.kept.title", "Services you update")}
			stamp={<FreshnessStamp {...stampOf(placements.freshness)} />}
			foot={t(
				"deployShip.kept.foot",
				"Changing an existing approval is a separate action on the service page.",
			)}
		>
			<ul className="list-disc pl-5 text-ui">
				{kept.map((row) => (
					<li key={`${row.target.deviceId}/${row.service.serviceId}`}>
						{keptLine(c, row, find(row))}
					</li>
				))}
			</ul>
		</Block>
	);
}

/* The step. */

/** A plan that ticks models needs a spending limit before it can be checked. */
function useSpendingDefault({
	draft,
	update,
}: Pick<FormProps, "draft" | "update">) {
	const needs = draft.approval.models.length > 0 && draft.spending === null;
	const { approval } = draft;
	useEffect(() => {
		if (needs) update({ spending: defaultSpending(approval) });
	}, [needs, approval, update]);
}

export function AccessCostStep(props: Readonly<DeployStepProps>) {
	const { plan, draft, update } = props;
	const { t } = useTranslation("devices");
	const fresh = useMemo(() => newServices(plan), [plan]);
	const kept = useMemo(() => keptServices(plan), [plan]);
	const approver = useApprover(plan, fresh);
	useSpendingDefault({ draft, update });
	const blocked = fresh.length > 0 && approver.blocked;
	const notOwner = approver.known && !approver.owner;
	const prepared =
		props.prepared ?? deployRunExtras(draft.deploymentId).prepared;
	const form: FormProps = {
		plan,
		draft,
		update,
		fresh,
		blocked,
		notOwner,
		prepared,
	};
	const editable = fresh.length > 0 || plan.targets.length === 0;
	return (
		<div className="flex min-w-0 flex-col gap-4">
			<WizardStepHeader
				step={6}
				total={8}
				title={t("deployShip.step.accessCost", "Access & cost")}
				lede={t(
					"deployShip.access.lede",
					"Online services need cloud access to reach {{app}}'s data. Each service on each device gets its own approval.",
					{ app: plan.app?.name ?? "" },
				)}
			/>
			{plan.targets.length === 0 ? (
				<Banner tone="info">
					{t(
						"deployShip.access.noTargets",
						"Pick devices in Where first; each one gets its own approval.",
					)}
				</Banner>
			) : null}
			{blocked ? (
				<ApproverGate plan={plan} approver={approver} fresh={fresh} />
			) : null}
			{fresh.length ? <ApproverBlock plan={plan} approver={approver} /> : null}
			{editable ? <CloudBlock {...form} /> : null}
			{editable && draft.approval.models.length ? (
				<SpendingBlock {...form} />
			) : null}
			{editable ? <WritesBlock {...form} /> : null}
			<KeptBlock plan={plan} kept={kept} />
		</div>
	);
}
