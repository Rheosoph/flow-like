"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Box,
	Check,
	ChevronDown,
	CircleCheck,
	CircleDashed,
	CloudUpload,
	Copy,
	Download,
	Ellipsis,
	ExternalLink,
	FileKey,
	Fingerprint,
	Hourglass,
	LockOpen,
	Shield,
	ShieldAlert,
	ShieldCheck,
	SlidersHorizontal,
	TriangleAlert,
	UserMinus,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import { groupFingerprint } from "../../../../lib/device-management/fingerprint";
import {
	orderCapabilities,
	presetOf,
} from "../../../../lib/device-management/model/permissions";
import type {
	HostIsolationMode,
	Presence,
} from "../../../../lib/device-management/model/types";
import {
	ACCESS_ENDING_SOON_S,
	type AccessRules,
	type GrantRow,
	MAX_GRANTS,
	codeCapabilities,
	connectionFileName,
	connectionFileText,
} from "../../../../lib/device-management/sharing";
import type {
	Capability,
	InventoryScope,
} from "../../../../lib/device-management/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { enumCopy, enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { type Gate, GateInline, GateNotice } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { PersonChip } from "../primitives/person-chip";
import { RequestedActual } from "../primitives/requested-actual";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import { useAppNames } from "../shell/attention-popover";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useOverlay } from "../workspace/overlay-store";
import { useAttentionState } from "../workspace/use-attention";
import { useGate } from "../workspace/use-gate";
import type { DeviceAccess, PersonNames } from "./use-access";

export const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
export const LINK_BUTTON = cx(LINK, "cursor-pointer text-left");
/** A device named inside a sentence: mono and semibold, underlined on hover only. */
export const OBJECT_LINK =
	"font-mono text-[0.92em] font-semibold whitespace-nowrap text-foreground no-underline hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
export const POPOVER =
	"rounded-lg border-border-strong bg-popover p-0 text-ui shadow-none backdrop-blur-none";
const MENU_ITEM = "text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";
/**
 * The app's base layer gives every table margins and every cell four borders;
 * `DvTable` rules rows only. Chips in a cell have 4 px corners and may wrap.
 */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0 [&_td_[data-slot=badge]]:h-auto [&_td_[data-slot=badge]]:min-h-5.5 [&_td_[data-slot=badge]]:rounded-md [&_td_[data-slot=badge]]:py-0.5 [&_td_[data-slot=badge]]:whitespace-normal [&_td_[data-slot=badge]>span]:whitespace-normal";

/** "in 14h", "in 19d": an end as a count; the area's relative times round to "tomorrow" and "next mo.". */
export function untilText(time: AreaTime, atS: number): string {
	const seconds = atS - time.nowS;
	if (Math.abs(seconds) < 3600) return time.ago(atS);
	const format = new Intl.RelativeTimeFormat(time.locale, {
		numeric: "always",
		style: "narrow",
	});
	return Math.abs(seconds) < 48 * 3600
		? format.format(Math.round(seconds / 3600), "hour")
		: format.format(Math.round(seconds / 86_400), "day");
}

/**
 * The foot note of a stepped sheet: where the person is and why Continue is
 * off. Phones show the position in the stepper's progress line only.
 */
export function WizardPosition({
	position,
	blocker,
}: Readonly<{ position: string; blocker: string | null }>) {
	return (
		<span data-wizard-position="">
			<span className="max-[720px]:hidden">{position}</span>
			{blocker ? (
				<span data-wizard-blocker="" className="text-warning">
					<span className="max-[720px]:hidden">{" · "}</span>
					{blocker}
				</span>
			) : null}
		</span>
	);
}

/** Hands a small public text file (request, connection file) to the browser's download. */
export function saveTextFile(fileName: string, text: string): void {
	const url = URL.createObjectURL(
		new Blob([text], { type: "application/json" }),
	);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = fileName;
	anchor.style.display = "none";
	document.body.append(anchor);
	anchor.click();
	anchor.remove();
	setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/* People. */

/** A person by account id: name and avatar once looked up; an account nobody can look up stays "Unknown account" with its id. */
export function AccessPerson({
	userId,
	showId = true,
	className,
}: Readonly<{ userId: string; showId?: boolean; className?: string }>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const identity = useUserIdentity(userId);
	if (identity.isResolved)
		return (
			<PersonChip
				name={identity.label}
				email={identity.user?.email}
				avatarUrl={identity.avatarUrl}
				you={userId === input.me}
				fullName
				className={className}
			/>
		);
	return (
		<span
			data-person-unknown=""
			className={cx("inline-flex min-w-0 flex-col", className)}
		>
			<span
				className="inline-flex min-w-0 items-center gap-1.5"
				title={t(
					"access.person.unknownTitle",
					"This account couldn't be looked up from here",
				)}
			>
				<span
					aria-hidden
					className="inline-flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[10px] font-semibold text-ink-2"
				>
					?
				</span>
				<span className="truncate">
					{identity.isPending
						? t("access.person.lookingUp", "Looking up…")
						: t("access.person.unknown", "Unknown account")}
				</span>
			</span>
			{showId ? (
				<span className="mt-0.5 truncate font-mono text-xs text-muted-foreground">
					{userId}
				</span>
			) : null}
		</span>
	);
}

/* Scope. */

export interface ScopeNames {
	/** "Whole device" · "App Invoice AI" · "Service invoice-extractor". */
	label(scope: InventoryScope): string;
	/** Inside a sentence: "the whole device" · "app Invoice AI" · "service invoice-extractor". */
	phrase(scope: InventoryScope): string;
	appName(appId: string): string;
}

export function useScopeNames(): ScopeNames {
	const { t } = useTranslation("devices");
	const appNames = useAppNames();
	return useMemo(() => {
		const appName = (appId: string) => appNames(appId) ?? appId;
		return {
			appName,
			label: (scope) =>
				scope.kind === "device"
					? enumLabel(t, "scopeKind", "device")
					: scope.kind === "project"
						? enumLabel(t, "scopeKind", "project", {
								name: appName(scope.project_id),
							})
						: enumLabel(t, "scopeKind", "placement", {
								name: scope.placement_id,
							}),
			phrase: (scope) =>
				scope.kind === "device"
					? t("access.scope.phraseDevice", "the whole device")
					: scope.kind === "project"
						? t("access.scope.phraseApp", "app {{name}}", {
								name: appName(scope.project_id),
							})
						: t("access.scope.phraseService", "service {{name}}", {
								name: scope.placement_id,
							}),
		};
	}, [t, appNames]);
}

const NAME_MARK = "\u0000";

/** "App {{name}}" with the name as a node, so the word order stays the translator's. */
function namedLabel(label: string, name: ReactNode): ReactNode {
	const [before, after = ""] = label.split(NAME_MARK);
	return (
		<>
			{before}
			{name}
			{after}
		</>
	);
}

/** "Whole device", "App {name}" linking to App › Devices, or "Service {id}" linking to the service. */
export function ScopeLabel({
	scope,
	deviceId,
}: Readonly<{ scope: InventoryScope; deviceId?: string }>) {
	const { t } = useTranslation("devices");
	const names = useScopeNames();
	const link = useRouteLink();
	if (scope.kind === "device") return <>{names.label(scope)}</>;
	if (scope.kind === "project")
		return (
			<span data-scope="app">
				{namedLabel(
					enumLabel(t, "scopeKind", "project", { name: NAME_MARK }),
					<a
						{...link(
							{ screen: "app-devices", by: "device" },
							{ scope: { kind: "app", appId: scope.project_id } },
						)}
						className={LINK}
					>
						{names.appName(scope.project_id)}
					</a>,
				)}
			</span>
		);
	return (
		<span data-scope="service">
			{namedLabel(
				enumLabel(t, "scopeKind", "placement", { name: NAME_MARK }),
				deviceId ? (
					<a
						{...link({
							screen: "service",
							deviceId,
							serviceId: scope.placement_id,
						})}
						className={cx(LINK, "font-mono")}
					>
						{scope.placement_id}
					</a>
				) : (
					<span className="font-mono">{scope.placement_id}</span>
				),
			)}
		</span>
	);
}

/* Permissions. */

function presetText(t: DevicesT, capabilities: readonly Capability[]) {
	const { preset, count } = presetOf(capabilities);
	return { preset: enumLabel(t, "preset", preset), count };
}

/** "Viewer · 3 permissions" as text, for sentences and summaries. */
export function permissionSummary(
	t: DevicesT,
	capabilities: readonly Capability[],
): string {
	const { preset, count } = presetText(t, capabilities);
	return t("devices:access.permissions.summary", {
		preset,
		count,
		defaultValue_one: "{{preset}} · {{count, number}} permission",
		defaultValue_other: "{{preset}} · {{count, number}} permissions",
	});
}

/** The same summary with the preset in bold, as table cells and reviews show it. */
export function PermissionSummary({
	capabilities,
}: Readonly<{ capabilities: readonly Capability[] }>) {
	const { t } = useTranslation("devices");
	const { preset, count } = presetText(t, capabilities);
	return (
		<Trans
			t={t}
			i18nKey="access.permissions.summaryBold"
			count={count}
			values={{ preset }}
			tOptions={{
				defaultValue_one: "<1>{{preset}}</1> · {{count, number}} permission",
				defaultValue_other: "<1>{{preset}}</1> · {{count, number}} permissions",
			}}
			components={{ 1: <b className="font-semibold" /> }}
		/>
	);
}

function CodeLine({
	capabilities,
	isolation,
	mine,
}: Readonly<{
	capabilities: readonly Capability[];
	isolation: HostIsolationMode | null | undefined;
	mine: boolean;
}>) {
	const { t } = useTranslation("devices");
	if (codeCapabilities(capabilities).length === 0) return null;
	const sandboxed = isolation === "required";
	const unknown = mine && (isolation === null || isolation === undefined);
	const Icon = sandboxed ? ShieldCheck : unknown ? Box : ShieldAlert;
	const text = mine
		? sandboxed
			? t(
					"access.permissions.runsYourCodeSandboxed",
					"Runs your code, sandboxed",
				)
			: unknown
				? t(
						"access.permissions.runsYourCodeUnknown",
						"Runs your code · sandbox shows once unlocked",
					)
				: t(
						"access.permissions.runsYourCodeFull",
						"Runs your code with the agent's full access",
					)
		: sandboxed
			? t("access.permissions.runsCodeSandboxed", "Runs code, sandboxed")
			: t(
					"access.permissions.runsCodeFull",
					"Runs code with full device access",
				);
	return (
		<span
			data-code-line={sandboxed ? "sandboxed" : unknown ? "unknown" : "full"}
			className={cx(
				"mt-1 flex items-start gap-1 text-xs",
				sandboxed || unknown ? "text-muted-foreground" : "text-warning",
			)}
		>
			<Icon aria-hidden className="mt-px size-3.25 shrink-0" />
			<span>{text}</span>
		</span>
	);
}

/** R10: the preset name and a count; the full list opens in a popover. */
export function PermissionsCell({
	capabilities,
	isolation,
	mine = false,
}: Readonly<{
	capabilities: readonly Capability[];
	isolation?: HostIsolationMode | null;
	/** The viewer's own permissions (recipient wording). */
	mine?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const { preset, count } = presetText(t, capabilities);
	const code = codeCapabilities(capabilities);
	return (
		<>
			<span data-permissions="">
				<b className="font-semibold">{preset}</b>
				{" · "}
				<Popover>
					<PopoverTrigger asChild>
						<button
							type="button"
							aria-haspopup="dialog"
							className={cx(LINK_BUTTON, "inline-flex items-center gap-0.5")}
						>
							{t("access.permissions.count", {
								count,
								defaultValue_one: "{{count, number}} permission",
								defaultValue_other: "{{count, number}} permissions",
							})}
							<ChevronDown aria-hidden className="size-3" />
						</button>
					</PopoverTrigger>
					<PopoverContent
						align="start"
						aria-label={t("access.permissions.popover", "Permissions")}
						className={cx(POPOVER, "w-[min(340px,calc(100vw-32px))]")}
					>
						<div className="border-b border-hairline px-3 py-2 text-ui font-semibold">
							{permissionSummary(t, capabilities)}
						</div>
						<ul className="flex max-h-[50vh] flex-col gap-2 overflow-auto px-3 py-2.5">
							{orderCapabilities(capabilities).map((capability) => {
								const copy = enumCopy(t, "capability", capability);
								const runs = code.includes(capability);
								const Icon = runs ? Box : Check;
								return (
									<li key={capability} className="flex items-start gap-2">
										<Icon
											aria-hidden
											className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
										/>
										<span className="min-w-0">
											<b className="font-medium">{copy.label}</b>
											{copy.explain ? (
												<span className="block text-xs text-muted-foreground">
													{copy.explain}
												</span>
											) : null}
										</span>
									</li>
								);
							})}
						</ul>
					</PopoverContent>
				</Popover>
			</span>
			<CodeLine capabilities={capabilities} isolation={isolation} mine={mine} />
		</>
	);
}

/* Status, dates, rules. */

const STATUS_LOOK = {
	active: { tone: "good", icon: CircleCheck },
	waiting: { tone: "info", icon: Hourglass },
	removing: { tone: "info", icon: Hourglass },
	expired: { tone: "unknown", icon: CircleDashed },
} as const;

export function GrantStatusCell({
	row,
	first,
}: Readonly<{ row: GrantRow; first: string }>) {
	const { t } = useTranslation("devices");
	const look = STATUS_LOOK[row.status];
	const label =
		row.status === "active"
			? enumLabel(t, "accessRules", "applied")
			: row.status === "waiting"
				? enumLabel(t, "accessRules", "waiting")
				: row.status === "removing"
					? t("access.status.removing", "Removed · waiting for device")
					: enumLabel(t, "grant", "expired");
	const version = row.pendingVersion;
	const sub =
		row.status === "waiting" && version !== undefined
			? row.isNew
				? t(
						"access.status.waitingNew",
						"In v{{n}}. {{name}} can't connect until the device applies it.",
						{ n: version, name: first },
					)
				: t(
						"access.status.waitingChanged",
						"In v{{n}}. The device still uses its previous rules until then.",
						{ n: version },
					)
			: row.status === "removing" && version !== undefined
				? t(
						"access.status.removingSub",
						"In v{{n}}. {{name}} can still connect until the device applies it.",
						{ n: version, name: first },
					)
				: null;
	return (
		<>
			<StatusChip
				tone={look.tone}
				icon={look.icon}
				data-grant-status={row.status}
			>
				{label}
			</StatusChip>
			{sub ? <CellSub>{sub}</CellSub> : null}
		</>
	);
}

/** An end date with its countdown; warned when it is close. */
export function EndsCell({
	expiresAt,
	soonS = ACCESS_ENDING_SOON_S,
}: Readonly<{ expiresAt: number; soonS?: number }>) {
	const time = useAreaTime();
	const left = expiresAt - time.nowS;
	const soon = left > 0 && left <= soonS;
	return (
		<>
			<span className="tabular-nums" title={time.abs(expiresAt)}>
				{time.at(expiresAt)}
			</span>
			<CellSub
				data-ends-soon={soon || undefined}
				className={cx("tabular-nums", soon && "text-warning")}
			>
				{soon ? (
					<TriangleAlert
						aria-hidden
						className="mr-1 inline size-3 align-[-1px]"
					/>
				) : null}
				{untilText(time, expiresAt)}
			</CellSub>
		</>
	);
}

/** How soon a device picks up newly saved rules, by its check-ins. */
export function waitSentence(
	t: DevicesT,
	presence: Presence,
	time: AreaTime,
): string {
	if (presence.kind === "never")
		return t(
			"devices:access.wait.never",
			"It hasn't checked in yet, so this applies after its first check-in.",
		);
	if (presence.kind === "offline")
		return presence.since === undefined
			? t(
					"devices:access.wait.offline",
					"It's offline, so this applies when it checks in again.",
				)
			: t(
					"devices:access.wait.offlineSince",
					"It's offline since {{since}}, so this applies when it checks in again.",
					{ since: time.at(presence.since) },
				);
	return t(
		"devices:access.wait.online",
		"It's online, so this usually applies within 5 minutes.",
	);
}

/** "edge-berlin-01 applies v6 usually within 5 minutes while it's online." */
export function appliesSentence(
	t: DevicesT,
	device: string,
	version: number,
	presence: Presence,
	time: AreaTime,
): string {
	const params = { device, n: version };
	if (presence.kind === "never")
		return t(
			"devices:access.applies.never",
			"{{device}} applies access rules v{{n}} after its first check-in.",
			params,
		);
	if (presence.kind === "offline")
		return presence.since === undefined
			? t(
					"devices:access.applies.offline",
					"{{device}} applies access rules v{{n}} when it checks in again.",
					params,
				)
			: t(
					"devices:access.applies.offlineSince",
					"{{device}} applies access rules v{{n}} when it checks in again (offline since {{since}}).",
					{ device, n: version, since: time.at(presence.since) },
				);
	return t(
		"devices:access.applies.online",
		"{{device}} applies access rules v{{n}}, usually within 5 minutes while it's online.",
		params,
	);
}

/** "Saved v5 → device has v5". */
export function RulesPair({
	rules,
	sub,
}: Readonly<{ rules: AccessRules; sub?: ReactNode }>) {
	const { t } = useTranslation("devices");
	return (
		<RequestedActual
			versions
			requested={t("access.rules.saved", "Saved v{{n}}", { n: rules.saved })}
			actual={t("access.rules.deviceHas", "device has v{{n}}", {
				n: rules.applied,
			})}
			label={t(
				"access.rules.pairLabel",
				"Saved version {{saved}}, the device has version {{applied}}",
				{ saved: rules.saved, applied: rules.applied },
			)}
			tone={rules.waiting ? "info" : "good"}
			icon={rules.waiting ? Hourglass : CircleCheck}
			sub={sub}
		/>
	);
}

const SANDBOX_LOOK = {
	required: { icon: ShieldCheck, warn: false },
	optional: { icon: Shield, warn: false },
	none: { icon: ShieldAlert, warn: true },
	unknown: { icon: CircleDashed, warn: false },
} as const;

/** What code-running permissions mean on this device. */
export function SandboxFact({
	isolation,
}: Readonly<{ isolation: HostIsolationMode | null }>) {
	const { t } = useTranslation("devices");
	const mode = isolation ?? "unknown";
	const look = SANDBOX_LOOK[mode];
	const Icon = look.icon;
	const label =
		mode === "unknown"
			? t("access.sandbox.unknown", "Unknown until read live")
			: enumLabel(t, "hostIsolation", mode);
	const hint = {
		required: t(
			"access.sandbox.requiredHint",
			"Code-running permissions run sandboxed.",
		),
		optional: t(
			"access.sandbox.optionalHint",
			"Not required, so code-running permissions need your confirmation.",
		),
		none: t(
			"access.sandbox.noneHint",
			"Code-running permissions give full device access. You confirm that when adding them.",
		),
		unknown: t(
			"access.sandbox.unknownHint",
			"Treated as no sandbox when you add code-running permissions.",
		),
	}[mode];
	return (
		<>
			<span
				data-sandbox={mode}
				className={cx(
					"inline-flex items-center gap-1",
					look.warn && "text-warning",
					mode === "unknown" && "text-muted-foreground",
				)}
			>
				<Icon aria-hidden className="size-3.25 shrink-0" />
				{label}
			</span>
			<span className="text-xs text-muted-foreground">{hint}</span>
		</>
	);
}

/* Locked or without keys: visible, disabled, with the reason (R7). */

export type KeysNeed = "locked" | "nokeys" | null;

export function keysNeedOf(device: Pick<DeviceAccess, "keys">): KeysNeed {
	const { state } = device.keys;
	if (state === "unlocked") return null;
	return state === "none" || state === "stale" ? "nokeys" : "locked";
}

/**
 * The one-line reason under a control that changes access: what the action's
 * own gate says (hub off, not the owner, owner keys on another computer, no
 * keys here), then the lock, because signing needs the owner key.
 */
export function useKeysGate(
	device: Pick<DeviceAccess, "keys" | "name" | "deviceId"> | undefined,
): Gate | null {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const gate = useGate("share_access", device?.deviceId);
	const need = device ? keysNeedOf(device) : null;
	const name = device?.name ?? "";
	if (!device) return null;
	if (!gate.ok)
		return {
			kind: gate.kind,
			reason: gateCopy(t, gate, { at: time.at }).inline,
		};
	if (need === "locked")
		return {
			kind: "locked",
			reason: t("access.gate.unlockFirst", "Unlock {{device}} first.", {
				device: name,
			}),
		};
	if (need === "nokeys")
		return {
			kind: "nokeys",
			reason: t("access.gate.noKeys", "No keys for {{device}} here.", {
				device: name,
			}),
		};
	return null;
}

/** Add people is refused once every access slot is taken. */
export function slotsGate(t: DevicesT, used: number | undefined): Gate | null {
	if (used === undefined || used < MAX_GRANTS) return null;
	return {
		kind: "policy",
		reason: gateCopy(t, {
			ok: false,
			gate: "G4",
			kind: "policy",
			hide: false,
			copy: {
				code: "access_slots_full",
				params: { count: used, max: MAX_GRANTS },
			},
		}).inline,
	};
}

export function UnlockButton({
	deviceId,
	size = "sm",
}: Readonly<{ deviceId: string; size?: "md" | "sm" | "xs" }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	return (
		<DvButton
			size={size}
			icon={LockOpen}
			onClick={() => overlay.openUnlock(deviceId)}
		>
			{t("access.action.unlock", "Unlock…")}
		</DvButton>
	);
}

export function RestoreKeysLink({
	deviceId,
	size = "sm",
}: Readonly<{ deviceId: string; size?: "md" | "sm" | "xs" }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	return (
		<DvButton size={size} icon={CloudUpload} asChild>
			<a {...link({ screen: "keys", focusDeviceId: deviceId })}>
				{t("access.action.restoreKeys", "Restore keys…")}
			</a>
		</DvButton>
	);
}

/** `GateNotice` leaves its paragraphs to the app's base layer, which gives every `p` 28 px leading. */
const NOTICE_TEXT = "[&_p:not([class])]:text-ui";

/** The block-form reason why access can't be read or changed on this computer right now. */
export function KeysNotice({
	device,
	className,
}: Readonly<{
	device: Pick<DeviceAccess, "keys" | "name" | "deviceId">;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const need = keysNeedOf(device);
	if (!need) return null;
	if (need === "locked") {
		const copy = gateCopy(t, {
			ok: false,
			gate: "G7",
			kind: "locked",
			hide: false,
			copy: { code: "locked_access", params: { device: device.name } },
		});
		return (
			<GateNotice
				kind="locked"
				title={copy.title}
				text={t(
					"access.gate.lockedText",
					"The people list is signed with your owner key and checked on this computer, so it shows once the device is unlocked. Changes are signed with the same key.",
				)}
				actions={<UnlockButton deviceId={device.deviceId} />}
				className={cx(NOTICE_TEXT, className)}
			/>
		);
	}
	const copy = gateCopy(t, {
		ok: false,
		gate: "G6",
		kind: "nokeys",
		hide: false,
		copy: { code: "no_keys_here", params: { device: device.name } },
	});
	return (
		<GateNotice
			kind="nokeys"
			title={copy.title}
			text={t(
				"access.gate.noKeysText",
				"Restore the keys from your account backup to see and change who has access.",
			)}
			actions={<RestoreKeysLink deviceId={device.deviceId} />}
			className={cx(NOTICE_TEXT, className)}
		/>
	);
}

/* The people of one device. */

export interface GrantHandlers {
	onRenew(row: GrantRow): void;
	onChange(row: GrantRow): void;
	onRemove(row: GrantRow): void;
}

function GrantMenu({
	device,
	row,
	name,
	handlers,
	disabled,
}: Readonly<{
	device: DeviceAccess;
	row: GrantRow;
	name: string;
	handlers: GrantHandlers;
	disabled: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { copy } = useCopy();
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton
					size="sm"
					variant="ghost"
					iconOnly
					icon={Ellipsis}
					aria-disabled={disabled || undefined}
					aria-label={t("access.people.more", "More for {{name}}", { name })}
				/>
			</DropdownMenuTrigger>
			{disabled ? null : (
				<DropdownMenuContent
					align="end"
					className="border-border-strong bg-popover shadow-none backdrop-blur-none"
				>
					<DropdownMenuItem
						onSelect={() => handlers.onChange(row)}
						className={MENU_ITEM}
					>
						<SlidersHorizontal aria-hidden />
						{t("access.action.changePermissions", "Change permissions…")}
					</DropdownMenuItem>
					<DropdownMenuItem
						onSelect={() => void copy(row.grant.controller_key.x)}
						className={MENU_ITEM}
					>
						<Fingerprint aria-hidden />
						{t("access.action.copyFingerprint", "Copy their key fingerprint")}
					</DropdownMenuItem>
					<DropdownMenuItem asChild className={MENU_ITEM}>
						<a
							{...link({
								screen: "device",
								deviceId: device.deviceId,
								tab: "access",
							})}
						>
							<ExternalLink aria-hidden />
							{t("access.action.openDeviceAccess", "Open device access")}
						</a>
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						onSelect={() => handlers.onRemove(row)}
						className={cx(MENU_ITEM, "text-critical focus:text-critical")}
					>
						<UserMinus aria-hidden />
						{t("access.action.removeNamed", "Remove {{name}}'s access…", {
							name,
						})}
					</DropdownMenuItem>
				</DropdownMenuContent>
			)}
		</DropdownMenu>
	);
}

function GrantActions({
	device,
	row,
	name,
	gate,
	handlers,
}: Readonly<{
	device: DeviceAccess;
	row: GrantRow;
	name: string;
	gate: Gate | null;
	handlers: GrantHandlers;
}>) {
	const { t } = useTranslation("devices");
	if (row.status === "removing")
		return (
			<span className="text-xs text-muted-foreground">
				{t(
					"access.people.nothingToDo",
					"Nothing to do until {{device}} applies v{{n}}.",
					{ device: device.name, n: row.pendingVersion ?? 0 },
				)}
			</span>
		);
	return (
		<>
			<span className="inline-flex flex-wrap items-center gap-1.5">
				<DvButton
					size="sm"
					aria-disabled={gate ? true : undefined}
					onClick={() => handlers.onRenew(row)}
				>
					{row.status === "expired"
						? t("access.action.addAgain", "Add again…")
						: t("access.action.renew", "Renew…")}
				</DvButton>
				<GrantMenu
					device={device}
					row={row}
					name={name}
					handlers={handlers}
					disabled={gate !== null}
				/>
			</span>
			{gate ? (
				<GateInline kind={gate.kind} className="mt-1">
					{gate.reason}
				</GateInline>
			) : null}
		</>
	);
}

/** "Linux" / "Mac" from what the device reported when it was read live; empty until then. */
export function platformLabel(
	t: DevicesT,
	platform: string | undefined,
): string {
	if (platform === "linux") return t("devices:access.platform.linux", "Linux");
	if (platform === "macos") return t("devices:access.platform.mac", "Mac");
	return platform ?? "";
}

/**
 * SPEC §5.2 column plan, with Status and Actions a little wider than the
 * spec's 14% so "Active on device" and the two controls fit beside a docked rail.
 */
export function GrantsTable({
	device,
	rows,
	names,
	gate,
	handlers,
}: Readonly<{
	device: DeviceAccess;
	rows: readonly GrantRow[];
	names: PersonNames;
	gate: Gate | null;
	handlers: GrantHandlers;
}>) {
	const { t } = useTranslation("devices");
	const labels = {
		person: t("access.people.col.person", "Person"),
		scope: t("access.people.col.scope", "Applies to"),
		permissions: t("access.people.col.permissions", "Permissions"),
		expires: t("access.people.col.expires", "Expires"),
		status: t("access.people.col.status", "Status"),
		actions: t("access.people.col.actions", "Actions"),
	};
	return (
		<DvTable
			label={t("access.people.tableLabel", "People with access to {{device}}", {
				device: device.name,
			})}
			cols={["19%", "15%", "21%", "13%", "16%", "16%"]}
			className={TABLE_RESET}
			head={
				<tr>
					<Th>{labels.person}</Th>
					<Th>{labels.scope}</Th>
					<Th>{labels.permissions}</Th>
					<Th>{labels.expires}</Th>
					<Th>{labels.status}</Th>
					<Th>{labels.actions}</Th>
				</tr>
			}
		>
			{rows.map((row) => {
				const person = names(row.grant.user_id);
				return (
					<Tr
						key={`${row.grant.grant_id}:${row.status}`}
						dim={row.status === "removing"}
						data-grant={row.grant.grant_id}
					>
						<Td label={labels.person} kind="name">
							<AccessPerson userId={row.grant.user_id} />
						</Td>
						<Td label={labels.scope}>
							<ScopeLabel scope={row.grant.scope} deviceId={device.deviceId} />
						</Td>
						<Td label={labels.permissions}>
							<PermissionsCell
								capabilities={row.grant.capabilities}
								isolation={device.isolation}
							/>
						</Td>
						<Td label={labels.expires}>
							<EndsCell expiresAt={row.grant.expires_at} />
						</Td>
						<Td label={labels.status}>
							<GrantStatusCell row={row} first={person.first} />
						</Td>
						<Td label={labels.actions} kind="act">
							<GrantActions
								device={device}
								row={row}
								name={person.name}
								gate={gate}
								handlers={handlers}
							/>
						</Td>
					</Tr>
				);
			})}
		</DvTable>
	);
}

/* The connection file an owner hands out. */

/**
 * A public key in a row: blocks of four with the case kept, the first four
 * blocks until opened. `IdRef group4` upper-cases and drops "-" and "_", so its
 * blocks would differ from what the other person reads out.
 */
export function KeyFingerprint({
	value,
	copyLabel,
}: Readonly<{ value: string; copyLabel: string }>) {
	const { t } = useTranslation("devices");
	const [revealed, setRevealed] = useState(false);
	const { copied, copy } = useCopy();
	const full = groupFingerprint(value);
	const blocks = full.split(" ");
	const short = blocks.length > 4 ? `${blocks.slice(0, 4).join(" ")} …` : full;
	return (
		<span
			data-idref=""
			data-key-fingerprint=""
			className="inline-flex max-w-full min-w-0 items-center gap-0.5 align-middle"
		>
			<button
				type="button"
				title={full}
				aria-expanded={revealed}
				onClick={() => setRevealed((current) => !current)}
				className={cx(
					"min-w-0 cursor-pointer rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px text-left font-mono text-xs text-ink-2 hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					revealed
						? "break-normal whitespace-normal"
						: "truncate whitespace-nowrap",
				)}
			>
				{revealed ? full : short}
			</button>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={
					copied ? t("access.fingerprint.copied", "Copied") : copyLabel
				}
				onClick={() => void copy(value)}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

/** A key as it is read out: blocks of four. */
export function FingerprintReadout({
	label,
	value,
	hint,
	children,
}: Readonly<{
	label: ReactNode;
	value: string;
	hint: ReactNode;
	children?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<div
			data-fingerprint=""
			className="flex flex-col gap-2 rounded-lg border border-border bg-surface-sunken px-3.5 py-3"
		>
			<span className="text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
				{label}
			</span>
			<span className="font-mono text-[15px]/6 font-medium tracking-[0.02em] [word-spacing:0.35em]">
				{groupFingerprint(value)}
			</span>
			<DvButton
				size="xs"
				variant="ghost"
				icon={copied ? Check : Copy}
				onClick={() => void copy(value)}
				className="self-start"
			>
				{copied
					? t("access.fingerprint.copied", "Copied")
					: t("access.fingerprint.copy", "Copy")}
			</DvButton>
			<span className="text-xs/4 text-muted-foreground">{hint}</span>
			{children}
		</div>
	);
}

/** "Download connection file…": the owner key fingerprint to read out, and the file itself. */
export function ConnectionFileSheet({
	device,
	open,
	onOpenChange,
}: Readonly<{
	device: DeviceAccess;
	open: boolean;
	onOpenChange(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const [downloadedAt, setDownloadedAt] = useState<number>();
	const time = useAreaTime();
	useEffect(() => {
		if (!open) setDownloadedAt(undefined);
	}, [open]);
	const receipt = open ? workspace.keys.receipt(device.deviceId) : undefined;
	const vault = open ? workspace.keys.vault(device.deviceId) : undefined;
	const ownerKey = vault?.ownerControllerKey
		? undefined
		: vault?.controllerPublic.controller_key;
	const fileName = connectionFileName(device.deviceId);
	const download = useCallback(() => {
		if (!receipt || !ownerKey) return;
		saveTextFile(fileName, connectionFileText(receipt, ownerKey));
		setDownloadedAt(Math.floor(workspace.clock.now() / 1000));
	}, [receipt, ownerKey, fileName, workspace]);
	const ready = !!receipt && !!ownerKey;
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			icon={FileKey}
			title={t("access.connection.title", "Connection file for {{device}}", {
				device: device.name,
			})}
			sub={t(
				"access.connection.subtitle",
				"Give it to the person who wants access",
			)}
			foot={
				<>
					<DvButton onClick={() => onOpenChange(false)}>
						{t("access.connection.close", "Close")}
					</DvButton>
					<DvButton
						variant="primary"
						icon={Download}
						aria-disabled={ready ? undefined : true}
						onClick={download}
					>
						{t("access.connection.download", "Download connection file")}
					</DvButton>
				</>
			}
		>
			<p className="text-ui">
				{t(
					"access.connection.intro",
					"The person imports this file in their app. It makes a key for them and an access request file, which they send back to you.",
				)}
			</p>
			{ready && ownerKey ? (
				<FingerprintReadout
					label={t(
						"access.connection.fingerprint",
						"Your owner key fingerprint",
					)}
					value={ownerKey.x}
					hint={t(
						"access.connection.fingerprintHint",
						"Read this out when they ask. They compare it with what their app shows before they send a request.",
					)}
				/>
			) : (
				<KeysNotice device={device} />
			)}
			<KeyValueList>
				<KvRow label={t("access.connection.device", "Device")}>
					<span className="font-mono">{device.name}</span>{" "}
					<IdRef
						id={device.deviceId}
						copyLabel={t("access.connection.copyDeviceId", "Copy device ID")}
					/>
				</KvRow>
				<KvRow label={t("access.connection.contains", "Contains")}>
					{t(
						"access.connection.containsText",
						"The device's signed registration and your owner public key. No secrets: with it, someone can only ask you for access.",
					)}
				</KvRow>
				<KvRow label={t("access.connection.file", "File")}>
					<span className="font-mono wrap-anywhere">{fileName}</span>
				</KvRow>
			</KeyValueList>
			{downloadedAt === undefined ? null : (
				<InlineResult tone="good">
					{t(
						"access.connection.downloaded",
						"Downloaded at {{time}}. Send the file to the person who asked.",
						{ time: time.clock(downloadedAt) },
					)}
				</InlineResult>
			)}
		</DvSheet>
	);
}
