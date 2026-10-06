"use client";

import { useTranslation } from "@flow-like/locales";
import { Box, TriangleAlert } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	KNOWN_CAPABILITIES,
	PERMISSION_PRESETS,
	type PermissionPreset,
	agentAccepts,
	hubAccepts,
	presetsFor,
	runsCode,
} from "../../../../lib/device-management/model/permissions";
import type { DeviceViewModel } from "../../../../lib/device-management/model/types";
import {
	ACCESS_DURATIONS_S,
	type AgentSupport,
	DEFAULT_ACCESS_S,
	type PermissionBlock,
	agentSupportOf,
	grantExpiry,
	offeredPreset,
	permissionBlock,
	presetCapabilities,
	rulesExpiryAfterSave,
	scopedCapabilities,
} from "../../../../lib/device-management/sharing";
import type {
	Capability,
	InventoryScope,
	ManagementGrant,
} from "../../../../lib/device-management/types";
import {
	type IBackendState,
	useBackend,
} from "../../../../state/backend-state";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { enumCopy, enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { CheckField, DvInput, Field } from "../primitives/form-fields";
import { GateInline } from "../primitives/gate-notice";
import { Segmented } from "../primitives/segmented";
import { cx } from "../primitives/tone";
import { useDeviceView } from "../workspace/use-fleet";
import type { DeviceAccess } from "./use-access";

/** What the owner chose for one person; the grant is derived from it. */
export interface PermissionDraft {
	scopeKind: InventoryScope["kind"];
	projectId: string;
	placementId: string;
	/** As ticked; permissions the scope can't hold are dropped when the grant is built. */
	capabilities: Capability[];
	/** Seconds from now; 0 keeps the current end (change permissions only). */
	durationS: number;
	groupId: string;
	groupVersion: string;
}

export function draftOf(previous?: ManagementGrant): PermissionDraft {
	if (!previous)
		return {
			scopeKind: "device",
			projectId: "",
			placementId: "",
			capabilities: [...PERMISSION_PRESETS.viewer],
			durationS: DEFAULT_ACCESS_S,
			groupId: "",
			groupVersion: "1",
		};
	const { scope } = previous;
	return {
		scopeKind: scope.kind,
		projectId: scope.kind === "device" ? "" : scope.project_id,
		placementId: scope.kind === "placement" ? scope.placement_id : "",
		capabilities: [...previous.capabilities],
		durationS: 0,
		groupId: previous.group_id ?? "",
		groupVersion: String(previous.group_version ?? 1),
	};
}

export function draftScope(draft: PermissionDraft): InventoryScope | undefined {
	if (draft.scopeKind === "device") return { kind: "device" };
	if (!draft.projectId) return undefined;
	if (draft.scopeKind === "project")
		return { kind: "project", project_id: draft.projectId };
	return draft.placementId
		? {
				kind: "placement",
				project_id: draft.projectId,
				placement_id: draft.placementId,
			}
		: undefined;
}

/** The permissions the grant will carry: what was ticked, minus what the scope or the agents can't hold. */
export function draftCapabilities(
	draft: PermissionDraft,
	support: AgentSupport,
	previous?: ManagementGrant,
): Capability[] {
	return scopedCapabilities(draft.capabilities, draft.scopeKind).filter(
		(capability) =>
			!permissionBlock(
				capability,
				draft.scopeKind,
				support,
				previous?.capabilities.includes(capability),
			),
	);
}

export type DraftProblem =
	| "no_permissions"
	| "choose_app"
	| "choose_service"
	| "group_version";

export function draftProblems(
	draft: PermissionDraft,
	support: AgentSupport,
	previous?: ManagementGrant,
): DraftProblem[] {
	const checks: [DraftProblem, boolean][] = [
		[
			"no_permissions",
			draftCapabilities(draft, support, previous).length === 0,
		],
		["choose_app", scopeIncomplete(draft, "project", draft.projectId)],
		["choose_service", scopeIncomplete(draft, "placement", draft.placementId)],
		["group_version", !rosterVersionValid(draft)],
	];
	return checks.flatMap(([problem, failed]) => (failed ? [problem] : []));
}

const scopeIncomplete = (
	draft: PermissionDraft,
	kind: InventoryScope["kind"],
	chosen: string,
) => draft.scopeKind === kind && !chosen;

function rosterVersionValid(draft: PermissionDraft): boolean {
	if (!draft.groupId) return true;
	const version = Number(draft.groupVersion);
	return Number.isSafeInteger(version) && version >= 1;
}

export function draftExpiry(
	draft: PermissionDraft,
	now: number,
	previous?: ManagementGrant,
): number {
	if (draft.durationS > 0) return grantExpiry(now, draft.durationS);
	return previous && previous.expires_at > now
		? previous.expires_at
		: grantExpiry(now, DEFAULT_ACCESS_S);
}

export function problemText(t: DevicesT, problem: DraftProblem): string {
	switch (problem) {
		case "no_permissions":
			return t(
				"devices:access.picker.problem.noPermissions",
				"Choose at least one permission.",
			);
		case "choose_app":
			return t("devices:access.picker.problem.chooseApp", "Choose an app.");
		case "choose_service":
			return t(
				"devices:access.picker.problem.chooseService",
				"Choose a service.",
			);
		case "group_version":
			return t(
				"devices:access.picker.problem.groupVersion",
				"Enter the roster version you approved, 1 or higher.",
			);
	}
}

/** `agentSupportOf` under the name add-people-sheet.tsx imports. */
export const certificateSupportOf = agentSupportOf;

export function durationLabel(t: DevicesT, seconds: number): string {
	if (seconds < 86_400)
		return t("devices:access.picker.hours", {
			count: Math.round(seconds / 3600),
			defaultValue_one: "{{count, number}} hour",
			defaultValue_other: "{{count, number}} hours",
		});
	return t("devices:access.picker.days", {
		count: Math.round(seconds / 86_400),
		defaultValue_one: "{{count, number}} day",
		defaultValue_other: "{{count, number}} days",
	});
}

export interface SelectOption {
	value: string;
	label: ReactNode;
}

/** The area's select look on the shadcn Select; `Field` passes `id` and the aria links. */
export function SelectControl({
	value,
	onValueChange,
	options,
	placeholder,
	className,
	...trigger
}: Readonly<{
	value: string;
	onValueChange(value: string): void;
	options: readonly SelectOption[];
	placeholder?: string;
	className?: string;
	id?: string;
	"aria-label"?: string;
	"aria-describedby"?: string;
	"aria-invalid"?: boolean;
	disabled?: boolean;
}>) {
	return (
		<Select
			value={value || undefined}
			onValueChange={onValueChange}
			disabled={trigger.disabled}
		>
			<SelectTrigger
				{...trigger}
				className={cx(
					"h-8.5 w-full max-w-full rounded-lg border-input bg-card px-2.5 text-[13px]/[18px] shadow-none hover:border-border-strong focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-ring dark:bg-card",
					className,
				)}
			>
				<SelectValue placeholder={placeholder} />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

interface AppOption {
	id: string;
	name: string;
}

type AppList = Awaited<ReturnType<IBackendState["appState"]["getApps"]>>;

function appOptionsOf(apps: AppList | undefined): AppOption[] {
	const options: AppOption[] = [];
	for (const [app, meta] of apps ?? [])
		options.push({ id: app.id, name: meta?.name ?? app.id });
	return options.sort(byName);
}

function byName(a: AppOption, b: AppOption): number {
	return a.name.localeCompare(b.name);
}

function useAppOptions(): AppOption[] {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	return useMemo(() => appOptionsOf(apps.data), [apps.data]);
}

interface KnownService {
	serviceId: string;
	projectId: string;
}

function knownServicesOf(
	services: DeviceViewModel["services"] | undefined,
): KnownService[] {
	if (!Array.isArray(services)) return [];
	const known: KnownService[] = [];
	for (const { serviceId, projectId } of services)
		known.push({ serviceId, projectId });
	return known;
}

function useKnownServices(deviceId: string | undefined): KnownService[] {
	const view = useDeviceView(deviceId, { watch: false });
	const services = view?.services;
	return useMemo(() => knownServicesOf(services), [services]);
}

type AgentBlock = Exclude<PermissionBlock, "device_only">;

/** Whether this device's agent is the reason for `block`. */
const CAUSES: Record<
	AgentBlock,
	(device: DeviceAccess, capability: Capability) => boolean
> = {
	certificates_unsupported: (device) => device.certificateSupport === false,
	certificates_unknown: (device) => device.certificateSupport === undefined,
	models_unsupported: (device, capability) =>
		device.features !== undefined && !agentAccepts(capability, device.features),
	models_unknown: (device) => device.features === undefined,
	hub_models_unsupported: (device, capability) =>
		!hubAccepts(capability, device.supportedCapabilities),
};

const AGENT_BLOCK_COPY: Record<
	AgentBlock,
	(t: DevicesT, device: string) => string
> = {
	certificates_unsupported: (t, device) =>
		t(
			"devices:access.picker.block.certificatesUnsupported",
			"{{device}}'s agent can't share this. Update the agent first.",
			{ device },
		),
	certificates_unknown: (t, device) =>
		t(
			"devices:access.picker.block.certificatesUnknown",
			"{{device}} hasn't been read live, so its agent's support for this is unknown. Connect to it once.",
			{ device },
		),
	models_unsupported: (t, device) =>
		t(
			"devices:models.use.access.blockUnsupported",
			"{{device}}'s agent doesn't host models. Update the agent first.",
			{ device },
		),
	hub_models_unsupported: (t) =>
		t(
			"devices:models.use.access.blockHubUnsupported",
			"This hub cannot save model permissions yet. Update the hub first.",
		),
	models_unknown: (t, device) =>
		t(
			"devices:models.use.access.blockUnknown",
			"{{device}} hasn't been read live, so whether its agent hosts models is unknown. Connect to it once.",
			{ device },
		),
};

function blockReason(
	t: DevicesT,
	block: PermissionBlock,
	capability: Capability,
	devices: readonly DeviceAccess[],
): string {
	if (block === "device_only")
		return t(
			"devices:access.picker.block.deviceOnly",
			"Only with whole-device access.",
		);
	let failing = devices[0];
	for (const device of devices)
		if (CAUSES[block](device, capability)) {
			failing = device;
			break;
		}
	return AGENT_BLOCK_COPY[block](t, failing?.name ?? "");
}

const HOST_OPERATIONS: readonly Capability[] = ["reboot", "update_agent"];
const SEG_ROW = "flex flex-wrap items-center gap-x-3 gap-y-1.5";

/** Every permission the agents accept, plus those ticked or already held (shown blocked when the agents don't), in display order. */
function shownCapabilities(
	draft: PermissionDraft,
	support: AgentSupport,
	previous: ManagementGrant | undefined,
): Capability[] {
	const kept = new Set([
		...draft.capabilities,
		...(previous?.capabilities ?? []),
	]);
	const shown = (capability: Capability) =>
		kept.has(capability) ||
		(agentAccepts(capability, support.features) &&
			hubAccepts(capability, support.supportedCapabilities));
	return KNOWN_CAPABILITIES.filter(shown);
}

function PermissionCheck({
	id,
	capability,
	draft,
	devices,
	support,
	previous,
	onToggle,
}: Readonly<{
	id: string;
	capability: Capability;
	draft: PermissionDraft;
	devices: readonly DeviceAccess[];
	support: AgentSupport;
	previous?: ManagementGrant;
	onToggle(capability: Capability, checked: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const copy = enumCopy(t, "capability", capability);
	const block = permissionBlock(
		capability,
		draft.scopeKind,
		support,
		previous?.capabilities.includes(capability),
	);
	const ticked = draft.capabilities.includes(capability);
	const macs = HOST_OPERATIONS.includes(capability)
		? devices.filter((device) => device.platform === "macos")
		: [];
	const RiskIcon = runsCode([capability]) ? Box : TriangleAlert;
	return (
		<div
			data-permission={capability}
			data-blocked={block ?? undefined}
			className={cx(
				"rounded-lg border border-hairline bg-card px-2.5 py-2",
				!block && ticked && "border-border-strong bg-row-selected",
				block && "bg-surface-sunken",
			)}
		>
			<CheckField
				id={id}
				checked={!block && ticked}
				disabled={block !== null}
				onCheckedChange={(checked) => onToggle(capability, checked)}
				className="flex w-full"
			>
				<span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
					<b className="font-medium">{copy.label}</b>
					{copy.note ? (
						<span className="inline-flex items-center gap-0.75 text-[11.5px]/4 font-medium text-warning">
							<RiskIcon aria-hidden className="size-3" />
							{copy.note}
						</span>
					) : null}
				</span>
				{copy.explain ? (
					<span className="mt-0.5 block text-xs/4 text-muted-foreground">
						{copy.explain}
					</span>
				) : null}
				{macs.length ? (
					<span className="mt-0.5 block text-xs/4 text-muted-foreground">
						{t(
							"access.picker.macNoEffect",
							"Has no effect on {{devices}} (Mac).",
							{ devices: macs.map((device) => device.name).join(", ") },
						)}
					</span>
				) : null}
				{block ? (
					<GateInline
						kind={block === "device_only" ? "policy" : "unsupported"}
						className="mt-0.5"
					>
						{blockReason(t, block, capability, devices)}
					</GateInline>
				) : null}
			</CheckField>
		</div>
	);
}

function ScopeFields({
	id,
	draft,
	devices,
	onChange,
}: Readonly<{
	id: string;
	draft: PermissionDraft;
	devices: readonly DeviceAccess[];
	onChange(patch: Partial<PermissionDraft>): void;
}>) {
	const { t } = useTranslation("devices");
	const apps = useAppOptions();
	const single = devices.length === 1 ? devices[0] : undefined;
	const services = useKnownServices(single?.deviceId);
	const onDevice = new Set(services.map((service) => service.projectId));
	const where = single
		? single.name
		: t("access.picker.theseDevices", "these devices");
	if (draft.scopeKind === "project")
		return (
			<Field
				id={`${id}-app`}
				label={enumLabel(t, "scopeKind", "project")}
				hint={t(
					"access.picker.appHint",
					"Covers every service of this app on the device, including ones deployed later.",
				)}
			>
				<SelectControl
					value={draft.projectId}
					onValueChange={(projectId) => onChange({ projectId })}
					placeholder={t("access.picker.chooseApp", "Choose an app")}
					options={apps.map((app) => ({
						value: app.id,
						label:
							!single || onDevice.has(app.id)
								? app.name
								: t(
										"access.picker.appNotOn",
										"{{app}} · not on {{where}} yet",
										{
											app: app.name,
											where,
										},
									),
					}))}
				/>
			</Field>
		);
	if (draft.scopeKind === "placement")
		return (
			<Field
				id={`${id}-service`}
				label={t("access.picker.serviceOn", "Service on {{device}}", {
					device: where,
				})}
				hint={
					services.length
						? undefined
						: t(
								"access.picker.noServices",
								"No services known on {{device}}. Unlock or connect it to list them.",
								{ device: where },
							)
				}
			>
				<SelectControl
					value={draft.placementId}
					onValueChange={(placementId) =>
						onChange({
							placementId,
							projectId:
								services.find((service) => service.serviceId === placementId)
									?.projectId ?? draft.projectId,
						})
					}
					placeholder={t("access.picker.chooseService", "Choose a service")}
					options={services.map((service) => ({
						value: service.serviceId,
						label: service.serviceId,
					}))}
				/>
			</Field>
		);
	return null;
}

function DurationField({
	id,
	draft,
	previous,
	onChange,
}: Readonly<{
	id: string;
	draft: PermissionDraft;
	previous?: ManagementGrant;
	onChange(patch: Partial<PermissionDraft>): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const now = Math.floor(time.nowS);
	const keep = previous && previous.expires_at > now ? previous : undefined;
	const options = [
		...(keep
			? [
					{
						value: "0",
						label: t(
							"access.picker.keepEnd",
							"Keep the current end ({{when}})",
							{ when: time.at(keep.expires_at) },
						),
					},
				]
			: []),
		...ACCESS_DURATIONS_S.map((seconds) => ({
			value: String(seconds),
			label: durationLabel(t, seconds),
		})),
	];
	const value =
		draft.durationS === 0 && !keep ? DEFAULT_ACCESS_S : draft.durationS;
	return (
		<Field
			id={`${id}-duration`}
			label={t("access.picker.lasts", "Access lasts")}
			hint={
				<>
					<span data-ends="" className="text-foreground">
						{t("access.picker.ends", "Ends {{when}}.", {
							when: time.at(draftExpiry(draft, now, previous)),
						})}
					</span>{" "}
					{t(
						"access.picker.lastsHint",
						"Up to 31 days. Saving re-signs the access rules until {{date}}, and access can't outlast them.",
						{ date: time.at(rulesExpiryAfterSave(now)) },
					)}
				</>
			}
		>
			<SelectControl
				value={String(value)}
				onValueChange={(next) => onChange({ durationS: Number(next) })}
				options={options}
				className="max-w-[320px]"
			/>
		</Field>
	);
}

/**
 * Step 3 of Add people and the body of Change permissions: applies to,
 * preset + permissions with their explanations, and how long access lasts.
 */
export function PermissionPicker({
	id,
	draft,
	onChange,
	devices,
	previous,
}: Readonly<{
	id: string;
	draft: PermissionDraft;
	onChange(next: PermissionDraft): void;
	/** The devices this person gets access to; Service needs exactly one. */
	devices: readonly DeviceAccess[];
	/** The access being changed. */
	previous?: ManagementGrant;
}>) {
	const { t } = useTranslation("devices");
	const patch = (change: Partial<PermissionDraft>) =>
		onChange({ ...draft, ...change });
	const support = agentSupportOf(devices);
	const { features, supportedCapabilities } = support;
	const effective = draftCapabilities(draft, support, previous);
	const preset = offeredPreset(effective, features, supportedCapabilities);
	const problems = draftProblems(draft, support, previous);
	const several = devices.length > 1;
	const narrow = draft.scopeKind !== "device";
	const offered = presetsFor("device", features, supportedCapabilities);
	const fitting = presetsFor(draft.scopeKind, features, supportedCapabilities);
	const scopeOptions = [
		{ value: "device" as const, label: enumLabel(t, "scopeKind", "device") },
		{ value: "project" as const, label: enumLabel(t, "scopeKind", "project") },
		{
			value: "placement" as const,
			label: enumLabel(t, "scopeKind", "placement"),
			disabled: several,
		},
	];
	const presetOptions = [
		...offered.map((value) => ({
			value: value as PermissionPreset | "custom",
			label: enumLabel(t, "preset", value),
			disabled: !fitting.includes(value),
		})),
		{ value: "custom" as const, label: enumLabel(t, "preset", "custom") },
	];
	const toggle = (capability: Capability, checked: boolean) =>
		patch({
			capabilities: checked
				? [...draft.capabilities, capability]
				: draft.capabilities.filter((held) => held !== capability),
		});
	const permissionProblems = problems.filter(
		(problem) => problem !== "group_version",
	);
	return (
		<div
			data-permission-picker=""
			className="@container/picker flex flex-col gap-4"
		>
			<div className="flex flex-col gap-1.5">
				<span className="text-[13px]/[18px] font-medium">
					{t("access.picker.appliesTo", "Applies to")}
				</span>
				<div className={SEG_ROW}>
					<Segmented
						label={t("access.picker.appliesTo", "Applies to")}
						options={scopeOptions}
						value={draft.scopeKind}
						onChange={(scopeKind) => patch({ scopeKind })}
					/>
					{several ? (
						<GateInline kind="policy">
							{t(
								"access.picker.serviceSingle",
								"Service needs a single device.",
							)}
						</GateInline>
					) : null}
				</div>
			</div>
			<ScopeFields id={id} draft={draft} devices={devices} onChange={patch} />
			<div className="flex flex-col gap-2">
				<span className="text-[13px]/[18px] font-medium">
					{t("access.picker.permissions", "Permissions")}
				</span>
				<div className={SEG_ROW}>
					<Segmented
						label={t("access.picker.preset", "Preset")}
						options={presetOptions}
						value={preset}
						onChange={(next) => {
							if (next !== "custom")
								patch({
									capabilities: [
										...presetCapabilities(
											next,
											features,
											supportedCapabilities,
										),
									],
								});
						}}
						wrap
					/>
					{narrow ? (
						<GateInline kind="policy">
							{offered.includes("model_user")
								? t(
										"devices:models.use.access.presetsNeedDevice",
										"Device admin and Model user need whole-device access.",
									)
								: t(
										"access.picker.adminNeedsDevice",
										"Device admin needs whole-device access.",
									)}
						</GateInline>
					) : null}
				</div>
				<div className="grid gap-x-2.5 gap-y-1.5 @min-[560px]/picker:grid-cols-2">
					{shownCapabilities(draft, support, previous).map((capability) => (
						<PermissionCheck
							key={capability}
							id={`${id}-cap-${capability}`}
							capability={capability}
							draft={draft}
							devices={devices}
							support={support}
							previous={previous}
							onToggle={toggle}
						/>
					))}
				</div>
				{permissionProblems.length ? (
					<p role="alert" className="text-xs text-critical">
						{permissionProblems
							.map((problem) => problemText(t, problem))
							.join(" ")}
					</p>
				) : null}
			</div>
			<DurationField
				id={id}
				draft={draft}
				previous={previous}
				onChange={patch}
			/>
			<details className="group rounded-lg border border-hairline px-3 py-2">
				<summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground">
					{t(
						"access.picker.groupSummary",
						"Advanced: record an approved group roster",
					)}
				</summary>
				<div className="mt-2.5 grid gap-3 @min-[560px]/picker:grid-cols-2">
					<Field
						id={`${id}-group`}
						label={t("access.picker.groupId", "Group ID")}
					>
						<DvInput
							mono
							autoComplete="off"
							maxLength={128}
							placeholder={t("access.picker.optional", "optional")}
							value={draft.groupId}
							onChange={(event) => patch({ groupId: event.target.value })}
						/>
					</Field>
					<Field
						id={`${id}-group-version`}
						label={t("access.picker.groupVersion", "Roster version")}
						error={
							problems.includes("group_version")
								? problemText(t, "group_version")
								: undefined
						}
					>
						<DvInput
							numeric
							inputMode="numeric"
							value={draft.groupVersion}
							onChange={(event) => patch({ groupVersion: event.target.value })}
						/>
					</Field>
				</div>
				<p className="mt-2 text-xs text-muted-foreground">
					{t(
						"access.picker.groupHint",
						"Records which roster you approved. People added to the group later get nothing from it.",
					)}
				</p>
			</details>
		</div>
	);
}
