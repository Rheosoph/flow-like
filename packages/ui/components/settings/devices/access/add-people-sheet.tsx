"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Activity,
	ChevronLeft,
	CircleCheck,
	CircleDashed,
	LoaderCircle,
	Lock,
	LockOpen,
	Minus,
	Plus,
	Server,
	ShieldAlert,
	SlidersHorizontal,
	TriangleAlert,
	UserPlus,
	WifiOff,
	X,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	PERMISSION_PRESETS,
	presetOf,
} from "../../../../lib/device-management/model/permissions";
import {
	DEFAULT_ACCESS_S,
	type GrantChangeKind,
	MAX_GRANTS,
	capabilityDiff,
	codeCapabilities,
	grantChangeKind,
	needsTrustConfirmation,
	sameScope,
	usedSlots,
} from "../../../../lib/device-management/sharing";
import type {
	Capability,
	InventoryScope,
	ManagementGrant,
} from "../../../../lib/device-management/types";
import { Checkbox } from "../../../ui/checkbox";
import { enumLabel } from "../copy/enum-labels";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	CheckField,
	DvInput,
	Field,
	SecretInput,
} from "../primitives/form-fields";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { PresenceChip, StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { WizardStepper } from "../primitives/wizard";
import { useActivityTray } from "../shell/activity-tray";
import { useOverlayStore } from "../workspace/overlay-store";
import { useAttentionState } from "../workspace/use-attention";
import {
	AccessPerson,
	KeyFingerprint,
	LINK_BUTTON,
	PermissionSummary,
	RestoreKeysLink,
	WizardPosition,
	appliesSentence,
	keysNeedOf,
	permissionSummary,
	platformLabel,
	untilText,
	useScopeNames,
	waitSentence,
} from "./access-parts";
import {
	RequestFileDrop,
	type RequestFileRow,
	fileErrorText,
} from "./import-file-sheet";
import {
	type PermissionDraft,
	PermissionPicker,
	SelectControl,
	certificateSupportOf,
	draftCapabilities,
	draftExpiry,
	draftOf,
	draftProblems,
	draftScope,
	problemText,
} from "./permission-picker";
import {
	type DeviceAccess,
	type PersonNames,
	type SaveAccessOutcome,
	saveErrorText,
	useAccessLocal,
	usePersonNames,
	useSaveAccessRules,
} from "./use-access";

export type AccessWizardStep =
	| "devices"
	| "files"
	| "perms"
	| "trust"
	| "review"
	| "result";

/** How the wizard opens; a new `id` starts a new run. */
export interface AccessWizardStart {
	id: string;
	mode: "add" | "change";
	step?: AccessWizardStep;
	deviceIds?: readonly string[];
	files?: readonly RequestFileRow[];
	/** `change`: the access being changed. */
	change?: { deviceId: string; grant: ManagementGrant };
	/** A device the link asked for that is locked or not usable. */
	noteDeviceId?: string;
}

interface FileState {
	ok: boolean;
	tone: "good" | "info" | "warning" | "critical";
	text: string;
	/** The file is for an owned device that isn't selected. */
	includeDeviceId?: string;
	needsDevice?: boolean;
}

interface WizardPerson {
	userId: string;
	deviceIds: string[];
	files: RequestFileRow[];
}

interface PlanEntry {
	deviceId: string;
	file: RequestFileRow;
	userId: string;
	previous?: ManagementGrant;
	capabilities: Capability[];
	scope?: InventoryScope;
	expiresAt: number;
	kind: GrantChangeKind;
	grant?: ManagementGrant;
}

type DeviceResult =
	| { status: "saved"; version: number; savedAt: number }
	| { status: "failed"; text: string; passwordRequired: boolean };

interface WizardContext {
	t: DevicesT;
	time: AreaTime;
	mode: "add" | "change";
	me: string;
	names: PersonNames;
	devices: ReadonlyMap<string, DeviceAccess>;
	deviceIds: readonly string[];
	files: readonly RequestFileRow[];
}

const previousGrant = (
	device: DeviceAccess | undefined,
	file: RequestFileRow,
): ManagementGrant | undefined =>
	device?.policy?.grants.find(
		(grant) => grant.grant_id === file.grantId && grant.user_id === file.userId,
	);

function fileStateOf(
	row: RequestFileRow,
	index: number,
	ctx: WizardContext,
): FileState {
	const { t, time } = ctx;
	if (row.error || !row.userId || !row.controllerKey)
		return { ok: false, tone: "critical", text: fileErrorText(t, row) };
	const who = ctx.names(row.userId);
	if (!row.deviceId)
		return {
			ok: false,
			tone: "warning",
			needsDevice: true,
			text: t(
				"devices:access.wizard.file.noDevice",
				"The file name doesn't say which device it's for. Choose the device their app made the key for.",
			),
		};
	const device = ctx.devices.get(row.deviceId);
	if (!device)
		return {
			ok: false,
			tone: "critical",
			text: t(
				"devices:access.wizard.file.notYours",
				"Made for a device that isn't yours. Only its owner can add people.",
			),
		};
	if (row.userId === ctx.me)
		return {
			ok: false,
			tone: "critical",
			text: t(
				"devices:access.wizard.file.self",
				"This is your own account. As the owner you already have every permission.",
			),
		};
	const grants = device.policy?.grants ?? [];
	const key = row.controllerKey.x;
	const clash = grants.find(
		(grant) =>
			grant.grant_id === row.grantId &&
			(grant.user_id !== row.userId || grant.controller_key.x !== key),
	);
	if (clash && ctx.mode === "add")
		return {
			ok: false,
			tone: "critical",
			text: t(
				"devices:access.wizard.file.clash",
				"Its request ID already belongs to {{other}}'s access on {{device}}. Ask {{who}} for a new access request.",
				{
					other: ctx.names(clash.user_id).name,
					device: device.name,
					who: who.first,
				},
			),
		};
	const duplicate = ctx.files
		.slice(0, index)
		.some(
			(other) =>
				!other.error &&
				other.userId === row.userId &&
				other.deviceId === row.deviceId,
		);
	if (duplicate)
		return {
			ok: false,
			tone: "warning",
			text: t(
				"devices:access.wizard.file.duplicate",
				"{{name}} is listed twice for {{device}}. Only the first file is used.",
				{ name: who.name, device: device.name },
			),
		};
	if (!ctx.deviceIds.includes(row.deviceId))
		return {
			ok: false,
			tone: "warning",
			includeDeviceId: row.deviceId,
			text: t(
				"devices:access.wizard.file.notSelected",
				"Made for {{device}}, which isn't selected. The key in it only works on that device.",
				{ device: device.name },
			),
		};
	const same = previousGrant(device, row);
	if (same && ctx.mode === "add")
		return {
			ok: true,
			tone: "info",
			text: t(
				"devices:access.wizard.file.renews",
				"{{name}} already has access to {{device}} ({{permissions}}, ends {{when}}). Adding again replaces those permissions and renews the access.",
				{
					name: who.name,
					device: device.name,
					permissions: enumLabel(
						t,
						"preset",
						presetOf(same.capabilities).preset,
					),
					when: time.at(same.expires_at),
				},
			),
		};
	if (!who.known)
		return {
			ok: true,
			tone: "warning",
			text: t(
				"devices:access.wizard.file.unknownAccount",
				"This account couldn't be looked up from here. Check the account ID with the person before you add it.",
			),
		};
	return {
		ok: true,
		tone: "good",
		text: t(
			"devices:access.wizard.file.ready",
			"Ready. Compare the fingerprint with what {{who}} reads out.",
			{ who: who.first },
		),
	};
}

function peopleOf(files: readonly RequestFileRow[]): WizardPerson[] {
	const people = new Map<string, WizardPerson>();
	for (const file of files) {
		if (!file.userId || !file.deviceId) continue;
		const person = people.get(file.userId) ?? {
			userId: file.userId,
			deviceIds: [],
			files: [],
		};
		person.deviceIds.push(file.deviceId);
		person.files.push(file);
		people.set(file.userId, person);
	}
	return [...people.values()];
}

function planOf(
	people: readonly WizardPerson[],
	draftFor: (person: WizardPerson) => PermissionDraft,
	devices: ReadonlyMap<string, DeviceAccess>,
	now: number,
): Map<string, PlanEntry[]> {
	const plan = new Map<string, PlanEntry[]>();
	for (const person of people) {
		const targets = person.deviceIds.flatMap((id) => devices.get(id) ?? []);
		const support = certificateSupportOf(targets);
		const draft = draftFor(person);
		for (const file of person.files) {
			const deviceId = file.deviceId as string;
			const previous = previousGrant(devices.get(deviceId), file);
			const capabilities = draftCapabilities(draft, support, previous);
			const scope = draftScope(draft);
			const expiresAt = draftExpiry(draft, now, previous);
			const grant: ManagementGrant | undefined =
				scope && file.controllerKey && file.grantId
					? {
							grant_id: file.grantId,
							user_id: person.userId,
							controller_key: file.controllerKey,
							scope,
							capabilities,
							expires_at: expiresAt,
							group_id: draft.groupId || null,
							group_version: draft.groupId ? Number(draft.groupVersion) : null,
						}
					: undefined;
			const entry: PlanEntry = {
				deviceId,
				file,
				userId: person.userId,
				...(previous ? { previous } : {}),
				capabilities,
				...(scope ? { scope } : {}),
				expiresAt,
				kind: grantChangeKind(previous, {
					scope: scope ?? { kind: "device" },
					capabilities,
					expires_at: expiresAt,
				}),
				...(grant ? { grant } : {}),
			};
			plan.set(deviceId, [...(plan.get(deviceId) ?? []), entry]);
		}
	}
	return plan;
}

const ackKey = (userId: string, deviceId: string) => `${userId}|${deviceId}`;

interface TrustItem {
	userId: string;
	deviceId: string;
	code: Capability[];
}

function trustOf(
	plan: ReadonlyMap<string, PlanEntry[]>,
	devices: ReadonlyMap<string, DeviceAccess>,
): TrustItem[] {
	const items: TrustItem[] = [];
	for (const [deviceId, entries] of plan)
		for (const entry of entries)
			if (
				needsTrustConfirmation(
					entry.previous?.capabilities,
					entry.capabilities,
					devices.get(deviceId)?.isolation,
				)
			)
				items.push({
					userId: entry.userId,
					deviceId,
					code: codeCapabilities(entry.capabilities),
				});
	return items;
}

function stepLabels(t: DevicesT): Record<AccessWizardStep, string> {
	return {
		devices: t("devices:access.wizard.step.devices", "Devices"),
		files: t("devices:access.wizard.step.files", "Request files"),
		perms: t("devices:access.wizard.step.perms", "Permissions"),
		trust: t("devices:access.wizard.step.trust", "Trust"),
		review: t("devices:access.wizard.step.review", "Review"),
		result: t("devices:access.wizard.step.result", "Saved"),
	};
}

/* Step 1: devices. */

function deviceFacts(t: DevicesT, device: DeviceAccess): string {
	const sandbox =
		device.isolation === "required"
			? t("devices:access.wizard.device.sandboxRequired", "sandbox required")
			: device.isolation === "none"
				? t("devices:access.wizard.device.noSandbox", "no sandbox")
				: device.isolation === "optional"
					? t(
							"devices:access.wizard.device.sandboxAvailable",
							"sandbox available",
						)
					: t("devices:access.wizard.device.sandboxUnknown", "sandbox unknown");
	const { rules, rows } = device;
	const sharing = !rules
		? t("devices:access.wizard.device.onlyYou", "only you so far")
		: rows
			? rules.waiting
				? t(
						"devices:access.wizard.device.rulesWaiting",
						"access rules v{{n}} (not applied yet) · {{used, number}} of {{max, number}} people",
						{ n: rules.saved, used: usedSlots(rows), max: MAX_GRANTS },
					)
				: t(
						"devices:access.wizard.device.rules",
						"access rules v{{n}} · {{used, number}} of {{max, number}} people",
						{ n: rules.saved, used: usedSlots(rows), max: MAX_GRANTS },
					)
			: t("devices:access.wizard.device.rulesVersion", "access rules v{{n}}", {
					n: rules.saved,
				});
	return [platformLabel(t, device.platform), sandbox, sharing]
		.filter(Boolean)
		.join(" · ");
}

function PresenceHint({ device }: Readonly<{ device: DeviceAccess }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { kind, since } = device.presence;
	if (kind === "offline")
		return (
			<span className="flex items-start gap-1 text-xs text-warning">
				<WifiOff aria-hidden className="mt-px size-3.25 shrink-0" />
				{since === undefined
					? t(
							"access.wizard.device.offline",
							"Offline. Changes apply when it checks in again.",
						)
					: t(
							"access.wizard.device.offlineSince",
							"Offline since {{since}}. Changes apply when it checks in again.",
							{ since: time.at(since) },
						)}
			</span>
		);
	if (kind === "never")
		return (
			<span className="flex items-start gap-1 text-xs text-muted-foreground">
				<CircleDashed aria-hidden className="mt-px size-3.25 shrink-0" />
				{t(
					"access.wizard.device.never",
					"Hasn't checked in yet. Changes apply after its first check-in.",
				)}
			</span>
		);
	return (
		<span className="flex items-start gap-1 text-xs text-muted-foreground">
			<CircleCheck aria-hidden className="mt-px size-3.25 shrink-0" />
			{t(
				"access.wizard.device.online",
				"Online. Changes usually apply within 5 minutes.",
			)}
		</span>
	);
}

function DeviceChoice({
	device,
	checked,
	onToggle,
	onUnlock,
}: Readonly<{
	device: DeviceAccess;
	checked: boolean;
	onToggle(checked: boolean): void;
	onUnlock(deviceId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const need = keysNeedOf(device);
	const id = `access-wizard-device-${device.deviceId}`;
	return (
		<div
			data-device-choice={device.deviceId}
			data-locked={need ?? undefined}
			className={cx(
				"flex items-start gap-3 rounded-lg border border-border bg-card px-3.5 py-3",
				need && "bg-surface-sunken",
			)}
		>
			<Checkbox
				id={id}
				checked={checked && !need}
				disabled={need !== null}
				onCheckedChange={(next) => onToggle(next === true)}
				className="mt-0.5 border-border-strong shadow-none data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			/>
			<Server
				aria-hidden
				className={cx(
					"mt-0.5 size-4 shrink-0 text-ink-2",
					need && "opacity-65",
				)}
			/>
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				<span className={cx("flex flex-col gap-1", need && "opacity-65")}>
					<label htmlFor={id} className="font-mono text-ui font-semibold">
						{device.name}
					</label>
					<span className="text-xs text-muted-foreground">
						{deviceFacts(t, device)}
					</span>
					<PresenceHint device={device} />
				</span>
				{need ? (
					<span className="mt-1 flex flex-col items-start gap-1.5">
						<GateInline kind={need}>
							{need === "locked"
								? t(
										"access.wizard.device.unlockFirst",
										"Unlock {{device}} first: access rules are signed with your owner key.",
										{ device: device.name },
									)
								: t(
										"access.wizard.device.noKeys",
										"This computer has no keys for {{device}}.",
										{ device: device.name },
									)}
						</GateInline>
						{need === "locked" ? (
							<DvButton
								size="xs"
								icon={LockOpen}
								onClick={() => onUnlock(device.deviceId)}
							>
								{t("access.action.unlock", "Unlock…")}
							</DvButton>
						) : (
							<RestoreKeysLink deviceId={device.deviceId} size="xs" />
						)}
					</span>
				) : null}
			</div>
		</div>
	);
}

const DEVICE_FILTER_FROM = 12;
const DEVICE_LIST_CAP = 30;

function DevicesStep({
	devices,
	selected,
	query,
	note,
	sharedNames,
	onQuery,
	onToggle,
	onUnlock,
}: Readonly<{
	devices: readonly DeviceAccess[];
	selected: readonly string[];
	query: string;
	note?: DeviceAccess;
	sharedNames: readonly string[];
	onQuery(query: string): void;
	onToggle(deviceId: string, checked: boolean): void;
	onUnlock(deviceId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const needle = query.trim().toLowerCase();
	const usable = (device: DeviceAccess) => (keysNeedOf(device) ? 1 : 0);
	const matches = devices
		.filter(
			(device) =>
				selected.includes(device.deviceId) ||
				!needle ||
				device.name.toLowerCase().includes(needle) ||
				device.deviceId.startsWith(needle),
		)
		.sort((a, b) => usable(a) - usable(b));
	const shown = matches.slice(0, DEVICE_LIST_CAP);
	return (
		<>
			{note && !selected.length ? (
				<Banner
					tone="info"
					icon={Lock}
					title={t("access.wizard.note.locked", "{{device}} is locked.", {
						device: note.name,
					})}
				>
					{t(
						"access.wizard.note.lockedText",
						"Unlock it below to add people to it.",
					)}
				</Banner>
			) : null}
			<p className="text-sm/5">
				{t(
					"access.wizard.devices.intro",
					"Choose the devices to share. Each person needs a request file for each device, because their app makes a separate key per device.",
				)}
			</p>
			{devices.length > DEVICE_FILTER_FROM ? (
				<Field
					id="access-wizard-device-filter"
					label={t(
						"access.wizard.devices.filter",
						"Filter {{count, number}} devices",
						{ count: devices.length },
					)}
					hint={
						matches.length > shown.length
							? t(
									"access.wizard.devices.filterHintMore",
									"{{selected, number}} selected · showing {{shown, number}} of {{matches, number}} matches",
									{
										selected: selected.length,
										shown: shown.length,
										matches: matches.length,
									},
								)
							: t(
									"access.wizard.devices.filterHint",
									"{{selected, number}} selected",
									{ selected: selected.length },
								)
					}
				>
					<DvInput
						autoComplete="off"
						placeholder={t(
							"access.wizard.devices.filterPlaceholder",
							"Name or device ID",
						)}
						value={query}
						onChange={(event) => onQuery(event.target.value)}
					/>
				</Field>
			) : null}
			<fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
				<legend className="sr-only">
					{t("access.wizard.step.devices", "Devices")}
				</legend>
				{shown.map((device) => (
					<DeviceChoice
						key={device.deviceId}
						device={device}
						checked={selected.includes(device.deviceId)}
						onToggle={(checked) => onToggle(device.deviceId, checked)}
						onUnlock={onUnlock}
					/>
				))}
			</fieldset>
			{sharedNames.length ? (
				<p className="text-xs/4 text-muted-foreground">
					<Trans
						t={t}
						i18nKey="access.wizard.devices.shared"
						count={sharedNames.length}
						values={{ names: sharedNames.join(", ") }}
						tOptions={{
							defaultValue_one:
								"<1>{{names}}</1> is shared with you. Only its owner can add people.",
							defaultValue_other:
								"{{count, number}} devices are shared with you (<1>{{names}}</1>). Only their owners can add people.",
						}}
						components={{ 1: <span className="font-mono text-[11.5px]" /> }}
					/>
				</p>
			) : null}
		</>
	);
}

/* Step 2: request files. */

const FLAG_TONE = {
	good: { cls: "text-good", icon: CircleCheck },
	info: { cls: "text-info", icon: CircleCheck },
	warning: { cls: "text-warning", icon: TriangleAlert },
	critical: { cls: "text-critical", icon: TriangleAlert },
} as const;

function FileRow({
	row,
	state,
	ctx,
	deviceOptions,
	onRemove,
	onDevice,
	onInclude,
}: Readonly<{
	row: RequestFileRow;
	state: FileState;
	ctx: WizardContext;
	deviceOptions: readonly { value: string; label: string }[];
	onRemove(): void;
	onDevice(deviceId: string): void;
	onInclude(deviceId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const flag = FLAG_TONE[state.tone];
	const FlagIcon = flag.icon;
	const device = row.deviceId ? ctx.devices.get(row.deviceId) : undefined;
	return (
		<li
			data-file-row={row.id}
			data-ok={state.ok}
			className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1.2fr)_auto] gap-x-4 gap-y-2 border-t border-hairline px-3.5 py-3 first:border-t-0 @max-[640px]/files:grid-cols-[minmax(0,1fr)_auto]"
		>
			<div className="min-w-0">
				{row.userId ? (
					<AccessPerson userId={row.userId} />
				) : (
					<span className="text-muted-foreground">
						{t("access.wizard.files.noPerson", "No person")}
					</span>
				)}
				<span
					className="mt-0.5 block truncate font-mono text-xs text-muted-foreground"
					title={row.file}
				>
					{row.file}
				</span>
			</div>
			<div className="min-w-0 @max-[640px]/files:col-start-1">
				<span className="block text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
					{t("access.wizard.files.forDevice", "For device")}
				</span>
				{device ? (
					<span className="font-mono">{device.name}</span>
				) : state.needsDevice ? (
					<SelectControl
						aria-label={t(
							"access.wizard.files.chooseDevice",
							"Device for {{file}}",
							{ file: row.file },
						)}
						value=""
						onValueChange={onDevice}
						placeholder={t("access.wizard.files.choose", "Choose the device")}
						options={deviceOptions}
						className="mt-1"
					/>
				) : (
					<span className="text-muted-foreground">
						{t("access.wizard.files.unknownDevice", "Unknown")}
					</span>
				)}
			</div>
			<div className="min-w-0 @max-[640px]/files:col-start-1">
				<span className="block text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
					{t("access.wizard.files.fingerprint", "Key fingerprint")}
				</span>
				{row.controllerKey ? (
					<KeyFingerprint
						value={row.controllerKey.x}
						copyLabel={t(
							"access.wizard.files.copyFingerprint",
							"Copy key fingerprint",
						)}
					/>
				) : (
					"–"
				)}
				{row.grantId ? (
					<span className="mt-1 block text-xs/4 text-muted-foreground">
						<Trans
							t={t}
							i18nKey="access.wizard.files.requestId"
							defaults="Request ID <1>{{id}}</1>"
							values={{ id: row.grantId.slice(0, 8) }}
							components={{
								1: (
									<span
										className="font-mono text-[11.5px]"
										title={row.grantId}
									/>
								),
							}}
						/>
					</span>
				) : null}
			</div>
			<DvButton
				size="xs"
				variant="ghost"
				iconOnly
				icon={X}
				aria-label={t("access.wizard.files.remove", "Remove {{file}}", {
					file: row.file,
				})}
				onClick={onRemove}
				className="col-start-4 row-start-1 @max-[640px]/files:col-start-2"
			/>
			<p
				data-flag={state.tone}
				className={cx(
					"col-span-full flex items-start gap-1.5 text-xs",
					flag.cls,
				)}
			>
				<FlagIcon aria-hidden className="mt-px size-3.25 shrink-0" />
				<span>
					{state.text}{" "}
					{state.includeDeviceId ? (
						<button
							type="button"
							className={LINK_BUTTON}
							onClick={() => onInclude(state.includeDeviceId as string)}
						>
							{t("access.wizard.files.include", "Include {{device}}", {
								device: device?.name ?? "",
							})}
						</button>
					) : null}
				</span>
			</p>
		</li>
	);
}

function FilesStep({
	ctx,
	states,
	ownedIds,
	onRows,
	onRemove,
	onDevice,
	onInclude,
}: Readonly<{
	ctx: WizardContext;
	states: readonly FileState[];
	ownedIds: readonly string[];
	onRows(rows: RequestFileRow[]): void;
	onRemove(id: string): void;
	onDevice(id: string, deviceId: string): void;
	onInclude(deviceId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const usable = states.filter((state) => state.ok).length;
	const deviceOptions = ctx.deviceIds.flatMap((id) => {
		const device = ctx.devices.get(id);
		return device ? [{ value: id, label: device.name }] : [];
	});
	return (
		<>
			<p className="text-sm/5">
				{t(
					"access.wizard.files.intro",
					"Each person imports your device's connection file in their app. It makes a new key for them and saves an access request file, which they send you.",
				)}
			</p>
			<RequestFileDrop
				id="access-wizard-files"
				ownedIds={ownedIds}
				fallbackDeviceId={
					ctx.deviceIds.length === 1 ? ctx.deviceIds[0] : undefined
				}
				onRows={onRows}
			/>
			{ctx.files.length ? (
				<>
					<ul
						aria-label={t("access.wizard.files.listLabel", "Imported files")}
						className="@container/files flex flex-col rounded-lg border border-border bg-card"
					>
						{ctx.files.map((row, index) => (
							<FileRow
								key={row.id}
								row={row}
								state={states[index] as FileState}
								ctx={ctx}
								deviceOptions={deviceOptions}
								onRemove={() => onRemove(row.id)}
								onDevice={(deviceId) => onDevice(row.id, deviceId)}
								onInclude={onInclude}
							/>
						))}
					</ul>
					<p className="text-xs text-muted-foreground">
						{t("access.wizard.files.usable", {
							usable,
							count: ctx.files.length,
							defaultValue_one:
								"{{usable, number}} of {{count, number}} file can be used. Files stay listed while you fix problems.",
							defaultValue_other:
								"{{usable, number}} of {{count, number}} files can be used. Files stay listed while you fix problems.",
						})}
					</p>
				</>
			) : null}
		</>
	);
}

/* Step 3: permissions per person. */

function PersonCard({
	index,
	person,
	draft,
	ctx,
	previous,
	onChange,
}: Readonly<{
	index: number;
	person: WizardPerson;
	draft: PermissionDraft;
	ctx: WizardContext;
	previous?: ManagementGrant;
	onChange(next: PermissionDraft): void;
}>) {
	const { t } = useTranslation("devices");
	const targets = person.deviceIds.flatMap((id) => ctx.devices.get(id) ?? []);
	const effective = draftCapabilities(
		draft,
		certificateSupportOf(targets),
		previous,
	);
	return (
		<section
			data-person-card={person.userId}
			aria-label={t(
				"access.wizard.perms.cardLabel",
				"Permissions for {{name}}",
				{
					name: ctx.names(person.userId).name,
				},
			)}
			className="rounded-lg border border-border bg-card"
		>
			<header className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-t-lg border-b border-hairline bg-surface-sunken px-3.5 py-2.5 text-sm/5">
				<AccessPerson userId={person.userId} showId={false} />
				<span className="text-muted-foreground">
					{t("access.wizard.perms.on", "on")}
				</span>
				<span className="font-mono text-[0.92em]">
					{targets.map((device) => device.name).join(", ")}
				</span>
				<span className="flex-1" />
				<span data-summary="" className="text-xs text-muted-foreground">
					{permissionSummary(t, effective)}
				</span>
			</header>
			<div className="p-3.5">
				<PermissionPicker
					id={`access-wizard-person-${index}`}
					draft={draft}
					onChange={onChange}
					devices={targets}
					previous={previous}
				/>
			</div>
		</section>
	);
}

/* Step 4: trust confirmation. */

function TrustStep({
	items,
	ctx,
	acks,
	onAck,
	onViewer,
}: Readonly<{
	items: readonly TrustItem[];
	ctx: WizardContext;
	acks: Readonly<Record<string, boolean>>;
	onAck(key: string, checked: boolean): void;
	onViewer(userId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const deviceIds = [...new Set(items.map((item) => item.deviceId))];
	return (
		<>
			<p className="text-sm/5">
				{t(
					"access.wizard.trust.intro",
					"Deploy & configure, Start, Restart and Change instance count run code. Confirm each person you trust with that on these devices.",
				)}
			</p>
			{deviceIds.map((deviceId) => {
				const device = ctx.devices.get(deviceId);
				if (!device) return null;
				const { isolation, isolationFacts, name } = device;
				const title =
					isolation === "none"
						? t("access.wizard.trust.noneTitle", "{{device}} has no sandbox.", {
								device: name,
							})
						: isolation === "optional"
							? t(
									"access.wizard.trust.optionalTitle",
									"{{device}} doesn't require a sandbox.",
									{ device: name },
								)
							: t(
									"access.wizard.trust.unknownTitle",
									"The app can't confirm that {{device}} requires a sandbox.",
									{ device: name },
								);
				const text =
					isolation === "none"
						? t(
								"access.wizard.trust.noneText",
								"Services there run as the agent, with full access to the device: its files, its network, other services and anything the agent's account can reach.",
							)
						: isolation === "optional"
							? t(
									"access.wizard.trust.optionalText",
									"Services can run sandboxed, but the device doesn't require it, so someone with Deploy & configure can run code as the agent with full device access.",
								)
							: t(
									"access.wizard.trust.unknownText",
									"{{device}} hasn't been read live, so its sandbox setting is unknown. Treat it as having no sandbox.",
									{ device: name },
								);
				return (
					<section
						key={deviceId}
						data-trust-device={deviceId}
						className="flex flex-col gap-3"
					>
						<Banner tone="warning" icon={ShieldAlert} title={title}>
							{text}
						</Banner>
						<KeyValueList>
							<KvRow label={t("access.wizard.trust.platform", "Platform")}>
								{device.platform ??
									t(
										"access.wizard.trust.platformUnknown",
										"Unknown until read live",
									)}
							</KvRow>
							<KvRow label={t("access.wizard.trust.sandbox", "Sandbox")}>
								{isolation === "none"
									? t("access.wizard.trust.sandboxNone", "Not available")
									: isolation === "optional"
										? t(
												"access.wizard.trust.sandboxOptional",
												"Available, not required",
											)
										: enumLabel(t, "hostIsolation", "unknown")}
								{isolationFacts?.reason ? (
									<span className="block text-xs text-muted-foreground">
										{t(
											"access.wizard.trust.sandboxReason",
											"Sandboxing needs Linux. On other systems, run the agent under a dedicated account.",
										)}
									</span>
								) : null}
							</KvRow>
							<KvRow label={t("access.wizard.trust.network", "Network")}>
								{t(
									"access.wizard.trust.networkText",
									"Sandboxed services still share the device's network. The sandbox doesn't filter loopback or link-local metadata addresses, such as cloud metadata endpoints.",
								)}
							</KvRow>
						</KeyValueList>
						{items
							.filter((item) => item.deviceId === deviceId)
							.map((item) => {
								const person = ctx.names(item.userId);
								const key = ackKey(item.userId, deviceId);
								return (
									<div
										key={key}
										data-trust-ack={key}
										className="flex flex-col gap-1.5 rounded-lg border border-warning-line bg-card px-3.5 py-3"
									>
										<p className="text-xs/4 text-muted-foreground">
											{t(
												"access.wizard.trust.wouldGet",
												"{{name}} would get {{permissions}} on {{device}}.",
												{
													name: person.name,
													permissions: item.code
														.map((capability) =>
															enumLabel(t, "capability", capability),
														)
														.join(", "),
													device: name,
												},
											)}
										</p>
										<CheckField
											id={`access-wizard-ack-${key}`}
											checked={acks[key] === true}
											onCheckedChange={(checked) => onAck(key, checked)}
										>
											{t(
												"access.wizard.trust.ack",
												"I understand {{name}} can run code on {{device}} with the agent's full access.",
												{ name: person.first, device: name },
											)}
										</CheckField>
										<button
											type="button"
											className={cx(LINK_BUTTON, "self-start text-ui")}
											onClick={() => onViewer(item.userId)}
										>
											{t(
												"access.wizard.trust.viewerInstead",
												"Give {{name}} Viewer instead",
												{ name: person.first },
											)}
										</button>
									</div>
								);
							})}
					</section>
				);
			})}
		</>
	);
}

/* Step 5: review. */

function changeKindLabel(t: DevicesT, kind: GrantChangeKind): string {
	return {
		new: t("devices:access.wizard.review.kindNew", "New access"),
		changed: t(
			"devices:access.wizard.review.kindChanged",
			"Changes existing access",
		),
		renewed: t(
			"devices:access.wizard.review.kindRenewed",
			"Renews existing access",
		),
		same: t(
			"devices:access.wizard.review.kindSame",
			"Same access, nothing changes",
		),
	}[kind];
}

const CHIP =
	"inline-flex h-5 items-center gap-0.75 rounded-md border border-hairline bg-surface-sunken px-1.5 text-xs whitespace-nowrap text-ink-2";
const CHIP_KIND = {
	added: "border-good-line bg-good-bg text-good",
	removed: "border-critical-line bg-critical-bg text-critical",
	kept: "",
} as const;

/** Every permission of the access as a chip: added ones marked, removed ones struck. */
function PermissionChips({
	before,
	after,
	label,
}: Readonly<{
	before: readonly Capability[];
	after: readonly Capability[];
	label: string;
}>) {
	const { t } = useTranslation("devices");
	const word = {
		added: t("access.wizard.review.chipAdded", "added"),
		removed: t("access.wizard.review.chipRemoved", "removed"),
	};
	return (
		<ul aria-label={label} className="mt-1 flex flex-wrap gap-1">
			{capabilityDiff(before, after).map(({ capability, kind }) => {
				const name = enumLabel(t, "capability", capability);
				const Sign = kind === "added" ? Plus : Minus;
				return (
					<li
						key={capability}
						data-k={kind}
						className={cx(CHIP, CHIP_KIND[kind])}
					>
						{kind === "kept" ? null : (
							<Sign aria-hidden className="size-2.75" />
						)}
						{kind === "removed" ? <s className="decoration-1">{name}</s> : name}
						{kind === "kept" ? null : (
							<span className="sr-only">{word[kind]}</span>
						)}
					</li>
				);
			})}
		</ul>
	);
}

function Becomes() {
	const { t } = useTranslation("devices");
	return (
		<>
			<span aria-hidden className="px-1 text-muted-foreground">
				→
			</span>
			<span className="sr-only">
				{t("access.wizard.review.becomes", "becomes")}
			</span>
		</>
	);
}

function ReviewEntry({
	entry,
	ctx,
	trusted,
}: Readonly<{ entry: PlanEntry; ctx: WizardContext; trusted: boolean }>) {
	const { t } = useTranslation("devices");
	const scopes = useScopeNames();
	const { time } = ctx;
	const device = ctx.devices.get(entry.deviceId);
	const { previous, scope } = entry;
	const scopeChanged =
		previous !== undefined &&
		scope !== undefined &&
		!sameScope(previous.scope, scope);
	const endsChanged =
		previous !== undefined && previous.expires_at !== entry.expiresAt;
	return (
		<div
			data-review-entry={`${entry.userId}|${entry.deviceId}`}
			className="grid gap-x-4 gap-y-2 border-t border-hairline px-3.5 py-3 first:border-t-0 @min-[560px]/review:grid-cols-[180px_minmax(0,1fr)]"
		>
			<div className="min-w-0">
				<AccessPerson userId={entry.userId} showId={false} />
				<span
					data-change-kind={entry.kind}
					className="mt-0.5 block text-xs/4 text-muted-foreground"
				>
					{changeKindLabel(t, entry.kind)}
				</span>
			</div>
			<dl className="m-0 grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-ui">
				<dt className="text-muted-foreground">
					{t("access.wizard.review.appliesTo", "Applies to")}
				</dt>
				<dd className="m-0 min-w-0">
					{scopeChanged && previous ? (
						<>
							<s className="text-muted-foreground">
								{scopes.label(previous.scope)}
							</s>
							<Becomes />
						</>
					) : null}
					{scope ? scopes.label(scope) : "–"}
				</dd>
				<dt className="text-muted-foreground">
					{t("access.wizard.review.permissions", "Permissions")}
				</dt>
				<dd className="m-0 min-w-0">
					<PermissionSummary capabilities={entry.capabilities} />
					<PermissionChips
						before={previous?.capabilities ?? []}
						after={entry.capabilities}
						label={t(
							"access.wizard.review.diffLabel",
							"Changes to {{name}}'s access",
							{ name: ctx.names(entry.userId).name },
						)}
					/>
				</dd>
				<dt className="text-muted-foreground">
					{t("access.wizard.review.ends", "Ends")}
				</dt>
				<dd className="m-0 min-w-0 tabular-nums">
					{endsChanged && previous ? (
						<>
							<span className="text-muted-foreground">
								{time.at(previous.expires_at)}
							</span>
							<Becomes />
						</>
					) : null}
					{time.at(entry.expiresAt)}{" "}
					<span className="text-muted-foreground">
						(
						{previous?.expires_at === entry.expiresAt
							? t("access.wizard.review.endsUnchanged", "{{when}}, unchanged", {
									when: untilText(time, entry.expiresAt),
								})
							: untilText(time, entry.expiresAt)}
						)
					</span>
				</dd>
			</dl>
			{trusted ? (
				<p className="col-span-full flex items-start gap-1 text-xs/4 text-warning">
					<ShieldAlert aria-hidden className="mt-0.5 size-3 shrink-0" />
					{t(
						"access.wizard.review.trusted",
						"Runs code with the agent's full access to {{device}}. You confirmed this in the Trust step.",
						{ device: device?.name ?? "" },
					)}
				</p>
			) : null}
		</div>
	);
}

function useReviewRows(
	plan: ReadonlyMap<string, PlanEntry[]>,
	ctx: WizardContext,
): ConsequenceRows {
	const scopes = useScopeNames();
	const { t, time } = ctx;
	const entries = [...plan.values()].flat();
	const deviceName = (deviceId: string) =>
		ctx.devices.get(deviceId)?.name ?? "";
	const what = entries.map((entry) => {
		const params = {
			name: ctx.names(entry.userId).name,
			permissions: permissionSummary(t, entry.capabilities),
			device: deviceName(entry.deviceId),
			scope: entry.scope ? scopes.phrase(entry.scope) : "",
			when: time.at(entry.expiresAt),
		};
		if (!entry.previous)
			return t(
				"devices:access.wizard.conseq.whatNew",
				"{{name}} gets {{permissions}} on {{device}}, {{scope}}, until {{when}}.",
				params,
			);
		if (entry.kind === "changed")
			return t(
				"devices:access.wizard.conseq.whatChanged",
				"{{name}} gets {{permissions}} on {{device}}, {{scope}}, replacing {{before}}, until {{when}}.",
				{
					name: params.name,
					permissions: params.permissions,
					device: params.device,
					scope: params.scope,
					when: params.when,
					before: permissionSummary(t, entry.previous.capabilities),
				},
			);
		return t(
			"devices:access.wizard.conseq.whatKeeps",
			"{{name}} keeps {{permissions}} on {{device}}, {{scope}}, until {{when}}.",
			params,
		);
	});
	const fresh = entries.filter((entry) => !entry.previous);
	const removed = entries.some((entry) =>
		entry.previous?.capabilities.some(
			(capability) => !entry.capabilities.includes(capability),
		),
	);
	const lostHistory = entries.filter((entry) =>
		(["logs", "metrics"] as const).some(
			(capability) =>
				entry.previous?.capabilities.includes(capability) &&
				!entry.capabilities.includes(capability),
		),
	);
	const who = [
		...fresh.map((entry) =>
			t(
				"devices:access.wizard.conseq.whoNew",
				"{{name}} sees {{device}} in their device list once it applies the rules.",
				{
					name: ctx.names(entry.userId).name,
					device: deviceName(entry.deviceId),
				},
			),
		),
		...(removed
			? [
					t(
						"devices:access.wizard.conseq.whoRemoved",
						"Removed permissions stop working when the device applies the rules.",
					),
				]
			: []),
		...lostHistory.map((entry) =>
			t(
				"devices:access.wizard.conseq.whoHistory",
				"{{name}} stops reading retained history on {{device}} for what was removed.",
				{
					name: ctx.names(entry.userId).first,
					device: deviceName(entry.deviceId),
				},
			),
		),
		t("devices:access.wizard.conseq.whoElse", "Nobody else's access changes."),
	];
	const when = [
		...[...plan.keys()].map((deviceId) => {
			const device = ctx.devices.get(deviceId);
			return device
				? appliesSentence(
						t,
						device.name,
						(device.rules?.saved ?? 0) + 1,
						device.presence,
						time,
					)
				: "";
		}),
		t(
			"devices:access.wizard.conseq.whenUntil",
			"Until then the previous rules apply. Access ends on its own at the end date.",
		),
	];
	return {
		what: what.join(" "),
		who: who.join(" "),
		stays: lostHistory.length
			? t(
					"devices:access.wizard.conseq.stays",
					"Everyone else keeps their permissions and end dates. Services keep running.",
				)
			: t(
					"devices:access.wizard.conseq.staysHistory",
					"Everyone else keeps their permissions and end dates. Services keep running. Retained history readers don't change.",
				),
		when: when.filter(Boolean).join(" "),
		undo: {
			reversible: true,
			text:
				ctx.mode === "change"
					? t(
							"devices:access.wizard.conseq.undoChange",
							"Change them back the same way.",
						)
					: t(
							"devices:access.wizard.conseq.undoAdd",
							"Change permissions or remove access from the People list. Each change saves a new version of the rules.",
						),
		},
	};
}

function PasswordFields({
	devices,
	passwords,
	onPassword,
}: Readonly<{
	devices: readonly DeviceAccess[];
	passwords: Readonly<Record<string, string>>;
	onPassword(deviceId: string, password: string): void;
}>) {
	const { t } = useTranslation("devices");
	if (!devices.length) return null;
	return (
		<div data-password-fields="" className="flex flex-col gap-3">
			{devices.map((device) => (
				<Field
					key={device.deviceId}
					id={`access-wizard-password-${device.deviceId}`}
					label={t(
						"access.wizard.review.password",
						"Device password for {{device}}",
						{ device: device.name },
					)}
					hint={t(
						"access.wizard.review.passwordHint",
						"The owner key isn't held by this unlock, so it's opened with the password for this one change.",
					)}
				>
					<SecretInput
						autoComplete="current-password"
						value={passwords[device.deviceId] ?? ""}
						onValueChange={(value) => onPassword(device.deviceId, value)}
					/>
				</Field>
			))}
		</div>
	);
}

interface ReviewDeviceProps {
	device: DeviceAccess;
	entries: readonly PlanEntry[];
	ctx: WizardContext;
	trust: readonly TrustItem[];
}

/** One device of the review: which version it gets and what changes for each person. */
function ReviewDevice(props: Readonly<ReviewDeviceProps>) {
	const { device, entries, ctx, trust } = props;
	const { t } = useTranslation("devices");
	const saved = device.rules?.saved ?? 0;
	const trusted = (entry: PlanEntry) =>
		trust.some(
			(item) =>
				item.userId === entry.userId && item.deviceId === device.deviceId,
		);
	return (
		<section
			data-review-device={device.deviceId}
			className="@container/review rounded-lg border border-border bg-card"
		>
			<header className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 rounded-t-lg border-b border-hairline bg-surface-sunken px-3.5 py-2.5 text-ui">
				<span className="font-mono text-[0.92em] font-semibold">
					{device.name}
				</span>
				<span className="text-muted-foreground">
					{saved
						? t(
								"access.wizard.review.version",
								"access rules v{{from}} → v{{to}}",
								{ from: saved, to: saved + 1 },
							)
						: t("access.wizard.review.firstVersion", "first access rules, v1")}
				</span>
				<span className="flex-1" />
				<PresenceChip
					kind={device.presence.kind}
					since={device.presence.since}
					short
				/>
			</header>
			{device.rules?.waiting ? (
				<p className="px-3.5 pt-2.5 text-xs text-muted-foreground">
					{t(
						"access.wizard.review.skips",
						"v{{from}} isn't applied yet. The device goes straight to v{{to}}.",
						{ from: saved, to: saved + 1 },
					)}
				</p>
			) : null}
			{entries.map((entry) => (
				<ReviewEntry
					key={`${entry.userId}|${entry.file.id}`}
					entry={entry}
					ctx={ctx}
					trusted={trusted(entry)}
				/>
			))}
		</section>
	);
}

interface ReviewStepProps {
	plan: ReadonlyMap<string, PlanEntry[]>;
	ctx: WizardContext;
	trust: readonly TrustItem[];
	results: Readonly<Record<string, DeviceResult>>;
	needPassword: readonly DeviceAccess[];
	passwords: Readonly<Record<string, string>>;
	onPassword(deviceId: string, password: string): void;
}

function ReviewStep(props: Readonly<ReviewStepProps>) {
	const { plan, ctx, trust, results, needPassword, passwords, onPassword } =
		props;
	const { t } = useTranslation("devices");
	const rows = useReviewRows(plan, ctx);
	const targets = [...plan.keys()].flatMap((id) => ctx.devices.get(id) ?? []);
	const held = targets.filter((device) => !needPassword.includes(device));
	const failures = Object.entries(results).flatMap(([deviceId, result]) =>
		result.status === "failed" ? [{ deviceId, text: result.text }] : [],
	);
	return (
		<>
			<p className="text-sm/5">
				{t(
					"access.wizard.review.intro",
					"Check what changes on each device. Nothing is saved until you select Save access rules.",
				)}
			</p>
			{failures.map((failure) => (
				<InlineResult key={failure.deviceId} tone="warning">
					{failure.text}
				</InlineResult>
			))}
			{targets.map((device) => (
				<ReviewDevice
					key={device.deviceId}
					device={device}
					entries={plan.get(device.deviceId) ?? []}
					ctx={ctx}
					trust={trust}
				/>
			))}
			<ConsequencePreview rows={rows} />
			<PasswordFields
				devices={needPassword}
				passwords={passwords}
				onPassword={onPassword}
			/>
			{held.length ? (
				<p
					data-signing-note=""
					className="flex items-start gap-1.5 text-xs text-muted-foreground"
				>
					<LockOpen aria-hidden className="mt-px size-3.25 shrink-0" />
					{t(
						"access.wizard.review.signed",
						"Signed with your owner key from the current unlock of {{devices}}. There's no separate password.",
						{ devices: held.map((device) => device.name).join(", ") },
					)}
				</p>
			) : null}
		</>
	);
}

/* Step 6: result per device. */

interface FailedResultProps {
	device: DeviceAccess;
	result: Extract<DeviceResult, { status: "failed" }>;
	passwords: Readonly<Record<string, string>>;
	onPassword(deviceId: string, password: string): void;
}

function FailedResult(props: Readonly<FailedResultProps>) {
	const { device, result, passwords, onPassword } = props;
	return (
		<li
			data-result-device={device.deviceId}
			data-result="failed"
			className="flex flex-col gap-2 rounded-lg border border-critical-line bg-card px-3.5 py-3 text-ui"
		>
			<span className="font-mono font-semibold">{device.name}</span>
			<InlineResult tone="critical">{result.text}</InlineResult>
			{result.passwordRequired ? (
				<PasswordFields
					devices={[device]}
					passwords={passwords}
					onPassword={onPassword}
				/>
			) : null}
		</li>
	);
}

interface SavedResultProps {
	device: DeviceAccess;
	result: Extract<DeviceResult, { status: "saved" }>;
	/** First names of the people this save concerns. */
	names: string;
	time: AreaTime;
}

/** Saved on the hub; follows the device until it applies the version. */
function SavedResult(props: Readonly<SavedResultProps>) {
	const { device, result, names, time } = props;
	const { t } = useTranslation("devices");
	const done = (device.rules?.applied ?? 0) >= result.version;
	const online =
		device.presence.kind === "online" || device.presence.kind === "late";
	const wait = waitSentence(t, device.presence, time);
	const waiting = online
		? t(
				"access.wizard.result.waitingOnline",
				"{{wait}} The app checks every 10 s.",
				{ wait },
			)
		: wait;
	return (
		<li
			data-result-device={device.deviceId}
			data-result={done ? "applied" : "waiting"}
			className="flex flex-col gap-1 rounded-lg border border-border bg-card px-3.5 py-3 text-ui"
		>
			<span className="flex flex-wrap items-center gap-2">
				<b className="font-mono font-semibold">{device.name}</b>
				<span className="text-muted-foreground">
					{t("access.wizard.result.version", "· saved v{{n}}", {
						n: result.version,
					})}
				</span>
				<span className="flex-1" />
				{done ? (
					<StatusChip tone="good" icon={CircleCheck}>
						{enumLabel(t, "accessRules", "applied")}
					</StatusChip>
				) : (
					<StatusChip tone="info" icon={LoaderCircle} spin={online}>
						{enumLabel(t, "accessRules", "waiting")}
					</StatusChip>
				)}
			</span>
			<span className="text-xs text-muted-foreground">
				{done
					? t(
							"access.wizard.result.applied",
							"{{device}} applied v{{n}}. {{names}} can use it now.",
							{ device: device.name, n: result.version, names },
						)
					: waiting}
			</span>
		</li>
	);
}

interface ResultStepProps {
	plan: ReadonlyMap<string, PlanEntry[]>;
	ctx: WizardContext;
	results: Readonly<Record<string, DeviceResult>>;
	passwords: Readonly<Record<string, string>>;
	onPassword(deviceId: string, password: string): void;
	onRetry(): void;
	retrying: boolean;
}

function firstNames(entries: readonly PlanEntry[], ctx: WizardContext) {
	const names = new Set<string>();
	for (const entry of entries) names.add(ctx.names(entry.userId).first);
	return [...names].join(", ");
}

function ResultStep(props: Readonly<ResultStepProps>) {
	const { plan, ctx, results, passwords, onPassword, onRetry, retrying } =
		props;
	const { t } = useTranslation("devices");
	const { time } = ctx;
	const outcomes = Object.values(results);
	const firstSaved = outcomes.find((result) => result.status === "saved");
	const failed = outcomes.filter((result) => result.status === "failed");
	const targets = [...plan.keys()].flatMap((id) => ctx.devices.get(id) ?? []);
	return (
		<>
			{firstSaved?.status === "saved" ? (
				<InlineResult tone="good">
					{t(
						"access.wizard.result.saved",
						"Access rules saved at {{time}} and signed with your owner key.",
						{ time: time.clock(firstSaved.savedAt) },
					)}
				</InlineResult>
			) : null}
			<ul aria-live="polite" className="flex flex-col gap-2">
				{targets.map((device) => {
					const result = results[device.deviceId];
					if (!result) return null;
					return result.status === "failed" ? (
						<FailedResult
							key={device.deviceId}
							device={device}
							result={result}
							passwords={passwords}
							onPassword={onPassword}
						/>
					) : (
						<SavedResult
							key={device.deviceId}
							device={device}
							result={result}
							names={firstNames(plan.get(device.deviceId) ?? [], ctx)}
							time={time}
						/>
					);
				})}
			</ul>
			{failed.length ? (
				<DvButton
					onClick={onRetry}
					busy={retrying}
					className="self-start"
					data-retry=""
				>
					{t("access.wizard.result.retry", {
						count: failed.length,
						defaultValue_one: "Try again for {{count, number}} device",
						defaultValue_other: "Try again for {{count, number}} devices",
					})}
				</DvButton>
			) : null}
			<p className="text-xs text-muted-foreground">
				{t(
					"access.wizard.result.note",
					"Until a device applies its new rules, the previous ones stay in force there. Imported request files that were used are cleared from Access requests.",
				)}
			</p>
		</>
	);
}

/* The wizard. */

/** What one device's save came to; a cancelled save leaves no result. */
function resultOf(
	t: DevicesT,
	outcome: SaveAccessOutcome,
	device: string,
	typedPassword: boolean,
): DeviceResult | undefined {
	if (outcome.status === "cancelled") return undefined;
	if (outcome.status === "done")
		return {
			status: "saved",
			version: outcome.result.version,
			savedAt: outcome.result.savedAt,
		};
	return {
		status: "failed",
		text: saveErrorText(t, outcome, device) ?? "",
		passwordRequired: typedPassword || outcome.status === "password_required",
	};
}

function initialDrafts(
	start: AccessWizardStart,
): Record<string, PermissionDraft> {
	if (!start.change) return {};
	const { grant } = start.change;
	return { [grant.user_id]: draftOf(grant) };
}

function initialFiles(start: AccessWizardStart): RequestFileRow[] {
	if (!start.change) return [...(start.files ?? [])];
	const { grant, deviceId } = start.change;
	return [
		{
			id: "change",
			file: "",
			bytes: 0,
			userId: grant.user_id,
			controllerKey: grant.controller_key,
			grantId: grant.grant_id,
			deviceId,
		},
	];
}

function WizardBody({
	start,
	devices,
	sharedNames,
	onClose,
}: Readonly<{
	start: AccessWizardStart;
	devices: readonly DeviceAccess[];
	sharedNames: readonly string[];
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const save = useSaveAccessRules();
	const local = useAccessLocal();
	const overlay = useOverlayStore((store) => store.overlay);
	const openUnlock = useOverlayStore((store) => store.openUnlock);
	const { mode } = start;

	const [step, setStep] = useState<AccessWizardStep>(
		start.step ?? (mode === "change" ? "perms" : "devices"),
	);
	const [deviceIds, setDeviceIds] = useState<string[]>(() =>
		start.change ? [start.change.deviceId] : [...(start.deviceIds ?? [])],
	);
	const [files, setFiles] = useState<RequestFileRow[]>(() =>
		initialFiles(start),
	);
	const [drafts, setDrafts] = useState(() => initialDrafts(start));
	const [acks, setAcks] = useState<Record<string, boolean>>({});
	const [passwords, setPasswords] = useState<Record<string, string>>({});
	const [query, setQuery] = useState("");
	const [saving, setSaving] = useState(false);
	const [results, setResults] = useState<Record<string, DeviceResult>>({});
	const [savedSteps, setSavedSteps] = useState<AccessWizardStep[]>();
	const [pendingInclude, setPendingInclude] = useState<string>();
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);

	const byId = useMemo(
		() => new Map(devices.map((device) => [device.deviceId, device])),
		[devices],
	);
	const names = usePersonNames(
		t,
		useMemo(
			() => [
				...files.flatMap((file) => file.userId ?? []),
				...devices.flatMap(
					(device) => device.policy?.grants.map((grant) => grant.user_id) ?? [],
				),
			],
			[files, devices],
		),
	);
	const unlockedIds = deviceIds.filter(
		(id) => byId.get(id)?.keys.state === "unlocked",
	);
	const ctx: WizardContext = {
		t,
		time,
		mode,
		me: input.me,
		names,
		devices: byId,
		deviceIds: unlockedIds,
		files,
	};

	// A device the person asked to include is added once its keys are open.
	useEffect(() => {
		if (!pendingInclude) return;
		if (byId.get(pendingInclude)?.keys.state !== "unlocked") return;
		setDeviceIds((current) =>
			current.includes(pendingInclude) ? current : [...current, pendingInclude],
		);
		setPendingInclude(undefined);
	}, [pendingInclude, byId]);

	const states = files.map((row, index) => fileStateOf(row, index, ctx));
	const usable = files.filter((_, index) => states[index]?.ok);
	const people = peopleOf(usable);
	const now = Math.floor(time.nowS);
	const draftFor = (person: WizardPerson) => {
		const chosen = drafts[person.userId];
		if (chosen) return chosen;
		const previous = person.files
			.map((file) => previousGrant(byId.get(file.deviceId as string), file))
			.find(Boolean);
		const draft = draftOf(previous);
		// Adding someone again renews their access unless another end is chosen.
		return previous ? { ...draft, durationS: DEFAULT_ACCESS_S } : draft;
	};
	const plan = planOf(people, draftFor, byId, now);
	const trust = trustOf(plan, byId);
	// After a save the plan is read against the new rules and needs no trust step any more: the steps taken stay.
	const steps: AccessWizardStep[] = savedSteps ?? [
		...(mode === "add" ? (["devices", "files"] as const) : []),
		"perms",
		...(trust.length ? (["trust"] as const) : []),
		"review",
		"result",
	];
	const current: AccessWizardStep = steps.includes(step)
		? step
		: step === "trust"
			? "review"
			: (steps[0] as AccessWizardStep);
	const index = steps.indexOf(current);
	const labels = stepLabels(t);

	const passwordAsked = (deviceId: string) => {
		const result = results[deviceId];
		return result?.status === "failed" && result.passwordRequired;
	};
	const needPassword = [...plan.keys()]
		.flatMap((id) => byId.get(id) ?? [])
		.filter((device) => !device.keys.canSign || passwordAsked(device.deviceId));
	const noteDevice = start.noteDeviceId
		? byId.get(start.noteDeviceId)
		: undefined;

	const blocker = ((): string | null => {
		if (current === "devices" && !unlockedIds.length)
			return t("access.wizard.block.devices", "Select at least one device.");
		if (current === "files" && !usable.length)
			return files.length
				? t(
						"access.wizard.block.filesUnusable",
						"None of these files can be used yet.",
					)
				: t(
						"access.wizard.block.files",
						"Import at least one access request file.",
					);
		if (current === "perms") {
			for (const person of people) {
				const targets = person.deviceIds.flatMap((id) => byId.get(id) ?? []);
				const previous = person.files
					.map((file) => previousGrant(byId.get(file.deviceId as string), file))
					.find(Boolean);
				const [problem] = draftProblems(
					draftFor(person),
					certificateSupportOf(targets),
					previous,
				);
				if (problem)
					return t("access.wizard.block.perms", "{{problem}} ({{name}})", {
						problem: problemText(t, problem),
						name: names(person.userId).first,
					});
			}
			if (
				mode === "change" &&
				[...plan.values()].flat().every((entry) => entry.kind === "same")
			)
				return t(
					"access.wizard.block.unchanged",
					"Change a permission, the scope or the end date first.",
				);
		}
		if (
			current === "trust" &&
			!trust.every((item) => acks[ackKey(item.userId, item.deviceId)])
		)
			return t(
				"access.wizard.block.trust",
				"Tick each confirmation, or choose fewer permissions.",
			);
		if (
			current === "review" &&
			needPassword.some((device) => !passwords[device.deviceId])
		)
			return t(
				"access.wizard.block.password",
				"Type the device password to sign.",
			);
		return null;
	})();

	const usedRequestIds = useCallback(
		(entries: readonly PlanEntry[]) =>
			local.requests
				.filter((request) =>
					entries.some(
						(entry) =>
							entry.userId === request.userId &&
							entry.file.controllerKey?.x === request.controllerKey.x &&
							(request.deviceId === undefined ||
								request.deviceId === entry.deviceId),
					),
				)
				.map((request) => request.id),
		[local.requests],
	);

	const saveAll = async () => {
		if (saving) return;
		setSaving(true);
		const next: Record<string, DeviceResult> = { ...results };
		const open = [...plan].filter(
			([deviceId]) => next[deviceId]?.status !== "saved",
		);
		for (const [deviceId, entries] of open) {
			const device = byId.get(deviceId);
			if (!device) continue;
			const password = passwords[deviceId];
			const outcome = await save({
				device,
				label: t("access.wizard.saveLabel", "Save access rules"),
				upserts: entries.flatMap((entry) => entry.grant ?? []),
				...(password ? { password } : {}),
			});
			if (!mounted.current) return;
			const result = resultOf(t, outcome, device.name, password !== undefined);
			if (result) next[deviceId] = result;
			if (result?.status === "saved")
				local.store.removeRequests(usedRequestIds(entries));
			setResults({ ...next });
		}
		setPasswords({});
		setSaving(false);
		if (Object.values(next).some((result) => result.status === "saved")) {
			setSavedSteps(steps);
			setStep("result");
		}
	};

	const goNext = () => {
		if (blocker || saving) return;
		if (current === "result") {
			onClose();
			return;
		}
		if (current === "review") {
			// A device that stopped requiring a sandbox while the review was open asks for trust first.
			if (trust.some((item) => !acks[ackKey(item.userId, item.deviceId)])) {
				setStep("trust");
				return;
			}
			void saveAll();
			return;
		}
		setStep(steps[index + 1] as AccessWizardStep);
	};

	const unlock = (deviceId: string) => {
		setPendingInclude(deviceId);
		openUnlock(deviceId);
	};
	const include = (deviceId: string) => {
		if (byId.get(deviceId)?.keys.state === "unlocked")
			setDeviceIds((ids) =>
				ids.includes(deviceId) ? ids : [...ids, deviceId],
			);
		else unlock(deviceId);
	};

	const changeTarget = start.change
		? byId.get(start.change.deviceId)
		: undefined;
	const changeName = start.change ? names(start.change.grant.user_id).name : "";

	let body: ReactNode;
	if (current === "devices")
		body = (
			<DevicesStep
				devices={devices}
				selected={unlockedIds}
				query={query}
				note={
					noteDevice && keysNeedOf(noteDevice) === "locked"
						? noteDevice
						: undefined
				}
				sharedNames={sharedNames}
				onQuery={setQuery}
				onToggle={(deviceId, checked) =>
					setDeviceIds((ids) =>
						checked
							? [...new Set([...ids, deviceId])]
							: ids.filter((id) => id !== deviceId),
					)
				}
				onUnlock={unlock}
			/>
		);
	else if (current === "files")
		body = (
			<FilesStep
				ctx={ctx}
				states={states}
				ownedIds={devices.map((device) => device.deviceId)}
				onRows={(rows) => setFiles((existing) => [...existing, ...rows])}
				onRemove={(id) =>
					setFiles((existing) => existing.filter((row) => row.id !== id))
				}
				onDevice={(id, deviceId) =>
					setFiles((existing) =>
						existing.map((row) => (row.id === id ? { ...row, deviceId } : row)),
					)
				}
				onInclude={include}
			/>
		);
	else if (current === "perms")
		body = (
			<>
				<p className="text-sm/5">
					{mode === "change" && changeTarget
						? t(
								"access.wizard.perms.introChange",
								"Change what {{name}} can do on {{device}}. The review shows what's added and removed.",
								{ name: changeName, device: changeTarget.name },
							)
						: t(
								"access.wizard.perms.intro",
								"Choose what each person can do. Start from a preset and adjust.",
							)}
				</p>
				{people.map((person, personIndex) => (
					<PersonCard
						key={person.userId}
						index={personIndex}
						person={person}
						draft={draftFor(person)}
						ctx={ctx}
						previous={person.files
							.map((file) =>
								previousGrant(byId.get(file.deviceId as string), file),
							)
							.find(Boolean)}
						onChange={(next) =>
							setDrafts((existing) => ({ ...existing, [person.userId]: next }))
						}
					/>
				))}
			</>
		);
	else if (current === "trust")
		body = (
			<TrustStep
				items={trust}
				ctx={ctx}
				acks={acks}
				onAck={(key, checked) =>
					setAcks((existing) => ({ ...existing, [key]: checked }))
				}
				onViewer={(userId) => {
					const person = people.find((entry) => entry.userId === userId);
					if (!person) return;
					setDrafts((existing) => ({
						...existing,
						[userId]: {
							...draftFor(person),
							capabilities: [...PERMISSION_PRESETS.viewer],
						},
					}));
				}}
			/>
		);
	else if (current === "review")
		body = (
			<ReviewStep
				plan={plan}
				ctx={ctx}
				trust={trust}
				results={results}
				needPassword={needPassword}
				passwords={passwords}
				onPassword={(deviceId, password) =>
					setPasswords((existing) => ({ ...existing, [deviceId]: password }))
				}
			/>
		);
	else
		body = (
			<ResultStep
				plan={plan}
				ctx={ctx}
				results={results}
				passwords={passwords}
				onPassword={(deviceId, password) =>
					setPasswords((existing) => ({ ...existing, [deviceId]: password }))
				}
				onRetry={() => void saveAll()}
				retrying={saving}
			/>
		);

	const nextLabel =
		current === "review"
			? t("access.wizard.save", "Save access rules")
			: current === "result"
				? t("access.wizard.done", "Done")
				: t("access.wizard.continue", "Continue");
	const hidden = overlay.kind === "unlock" || overlay.kind === "unlock_several";
	return (
		<DvSheet
			open={!hidden}
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			wide
			closeOnOutside={false}
			icon={mode === "add" ? UserPlus : SlidersHorizontal}
			title={
				mode === "add"
					? t("access.wizard.titleAdd", "Add people")
					: t("access.wizard.titleChange", "Change {{name}}'s permissions", {
							name: changeName,
						})
			}
			sub={
				mode === "add"
					? t(
							"access.wizard.subAdd",
							"Nothing is saved until the last step. Changes are signed with your owner key.",
						)
					: changeTarget && start.change
						? t(
								"access.wizard.subChange",
								"{{device}} · {{permissions}} · ends {{when}}",
								{
									device: changeTarget.name,
									permissions: permissionSummary(
										t,
										start.change.grant.capabilities,
									),
									when: time.at(start.change.grant.expires_at),
								},
							)
						: undefined
			}
			footNote={
				current === "result" ? (
					t(
						"access.wizard.footSaved",
						"Saved. Progress also shows in Activity.",
					)
				) : (
					<WizardPosition
						position={t(
							"access.wizard.position",
							"Step {{n, number}} of {{count, number}} · {{step}}",
							{ n: index + 1, count: steps.length, step: labels[current] },
						)}
						blocker={blocker}
					/>
				)
			}
			foot={
				<>
					{index > 0 && current !== "result" ? (
						<DvButton
							icon={ChevronLeft}
							aria-disabled={saving || undefined}
							onClick={() => setStep(steps[index - 1] as AccessWizardStep)}
						>
							{t("access.wizard.back", "Back")}
						</DvButton>
					) : null}
					{current === "result" ? (
						<DvButton
							icon={Activity}
							onClick={() => {
								onClose();
								useActivityTray.getState().setOpen(true);
							}}
						>
							{t("access.wizard.follow", "Follow in activity")}
						</DvButton>
					) : (
						<DvButton onClick={onClose}>
							{t("access.wizard.cancel", "Cancel")}
						</DvButton>
					)}
					<DvButton
						variant="primary"
						busy={saving}
						aria-disabled={blocker ? true : undefined}
						onClick={goNext}
					>
						{nextLabel}
					</DvButton>
				</>
			}
		>
			<WizardStepper
				steps={steps.map((id) => labels[id])}
				current={index}
				label={t("access.wizard.steps", "Steps")}
			/>
			{body}
		</DvSheet>
	);
}

/**
 * Add people (steps Devices → Request files → Permissions → Trust → Review →
 * Saved) and, with `mode: "change"`, the same from Permissions on for one
 * person. Hidden while the Unlock sheet is up, so sheets never stack.
 */
export function AccessWizard({
	start,
	devices,
	sharedNames = [],
	onClose,
}: Readonly<{
	start: AccessWizardStart | null;
	/** Every active device the viewer owns. */
	devices: readonly DeviceAccess[];
	sharedNames?: readonly string[];
	onClose(): void;
}>) {
	if (!start) return null;
	return (
		<WizardBody
			key={start.id}
			start={start}
			devices={devices}
			sharedNames={sharedNames}
			onClose={onClose}
		/>
	);
}

export function AddPeopleSheet(
	props: Readonly<{
		start: AccessWizardStart | null;
		devices: readonly DeviceAccess[];
		sharedNames?: readonly string[];
		onClose(): void;
	}>,
) {
	return <AccessWizard {...props} />;
}
