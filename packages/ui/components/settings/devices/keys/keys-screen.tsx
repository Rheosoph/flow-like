"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleDashed,
	CloudOff,
	CloudUpload,
	FileUp,
	KeyRound,
	Laptop,
	LoaderCircle,
	Lock,
	LockOpen,
	type LucideIcon,
	RotateCcw,
	ShieldCheck,
	TriangleAlert,
} from "lucide-react";
import {
	Fragment,
	type ReactNode,
	useCallback,
	useEffect,
	useState,
} from "react";
import type {
	DevicesRoute,
	KeysRoute,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block, PageHeader } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { SwitchField } from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { Headline } from "../primitives/headline";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { TONE_TEXT, type Tone, cx } from "../primitives/tone";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useAccessChangePassword, useKeyChip } from "../workspace/use-keys";
import { AccountBackupPanel, RESTORE_RESULT } from "./account-backup-panel";
import { GuideForgotPassword } from "./guide-forgot-password";
import { GuideNewComputer } from "./guide-new-computer";
import { type KeyFlowsApi, useKeyFlows } from "./key-flows";
import { KvHint, Mono, MonoNames, useNames } from "./key-parts";
import { useKeyFileLog, useKeyResults } from "./key-store";
import type { KeyGroup, KeyRow, KeysModel } from "./keys-model";
import { KeysTable, LocalOnlyKeys } from "./keys-table";
import {
	PersistenceNotice,
	StorageResult,
	storageAtRisk,
	storageHint,
	storageLabel,
} from "./persistence-notice";
import { type KeysRead, useKeysModel } from "./use-keys-model";

const KEYS_HOME: KeysRoute = { screen: "keys" };

function hostOf(origin: string): string {
	try {
		return new URL(origin).host;
	} catch {
		return origin;
	}
}

/** Device names as links that focus their row on this page, joined as a list. */
function FocusNames({ rows }: Readonly<{ rows: readonly KeyRow[] }>) {
	const { i18n } = useTranslation("devices");
	const link = useRouteLink();
	const parts = new Intl.ListFormat(i18n?.language ?? "en", {
		type: "conjunction",
	}).formatToParts(rows.map((row) => row.deviceId));
	const byId = new Map(rows.map((row) => [row.deviceId, row]));
	return (
		<>
			{parts.map((part, index) => {
				const row = part.type === "element" ? byId.get(part.value) : undefined;
				return row ? (
					<a
						key={row.deviceId}
						{...link(
							{ screen: "keys", focusDeviceId: row.deviceId },
							{ replace: true },
						)}
						className="font-mono text-[0.92em] font-semibold text-foreground no-underline hover:underline"
					>
						{row.name}
					</a>
				) : (
					// biome-ignore lint/suspicious/noArrayIndexKey: list separators have no identity
					<Fragment key={`sep-${index}`}>{part.value}</Fragment>
				);
			})}
		</>
	);
}

type RiskKind =
	| "never"
	| "pending"
	| "oldpw"
	| "hub_newer"
	| "out_of_date"
	| "lost";

const RISK_ORDER: readonly RiskKind[] = [
	"never",
	"pending",
	"oldpw",
	"hub_newer",
	"out_of_date",
	"lost",
];

/** One headline sentence per risk; the keys are literal so the extractor sees each one. */
function RiskSentence({
	kind,
	rows,
}: Readonly<{ kind: RiskKind; rows: readonly KeyRow[] }>) {
	const { t } = useTranslation("devices");
	const components = { 1: <FocusNames rows={rows} /> };
	const one = rows.length === 1;
	switch (kind) {
		case "never":
			return one ? (
				<Trans
					t={t}
					i18nKey="keys.headline.neverOne"
					defaults="<1/>'s keys exist only here."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.headline.neverMany"
					defaults="The keys for <1/> exist only here."
					components={components}
				/>
			);
		case "pending":
			return one ? (
				<Trans
					t={t}
					i18nKey="keys.headline.pendingOne"
					defaults="<1/>'s latest backup hasn't reached your account."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.headline.pendingMany"
					defaults="The latest backups of <1/> haven't reached your account."
					components={components}
				/>
			);
		case "oldpw":
			return one ? (
				<Trans
					t={t}
					i18nKey="keys.headline.oldpwOne"
					defaults="<1/>'s account backup still opens with the old password."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.headline.oldpwMany"
					defaults="The account backups of <1/> still open with the old password."
					components={components}
				/>
			);
		case "hub_newer":
			return one ? (
				<Trans
					t={t}
					i18nKey="keys.headline.hubNewerOne"
					defaults="Your account has a newer backup of <1/>'s keys than this computer."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.headline.hubNewerMany"
					defaults="Your account has newer backups than this computer for <1/>."
					components={components}
				/>
			);
		case "out_of_date":
			return one ? (
				<Trans
					t={t}
					i18nKey="keys.headline.outOfDateOne"
					defaults="<1/>'s account backup is older than the keys here."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.headline.outOfDateMany"
					defaults="The account backups of <1/> are older than the keys here."
					components={components}
				/>
			);
		default:
			return (
				<Trans
					t={t}
					i18nKey="keys.headline.noKeys"
					defaults="There are no keys for <1/> here or on your account."
					components={components}
				/>
			);
	}
}

const holdsLead = (t: DevicesT, count: number) =>
	t("devices:keys.headline.lead", {
		count,
		defaultValue_one: "This computer holds keys for {{count, number}} device.",
		defaultValue_other:
			"This computer holds keys for {{count, number}} devices.",
	});

/**
 * Before the device list answered (R6): what this computer knows on its own.
 * No device counts as "no keys" or "not backed up" from a list that isn't there.
 */
function pendingListHeadline(
	t: DevicesT,
	read: KeysRead,
): { lead: string; rest: ReactNode } {
	const held = read.local.vaults.length;
	const failed = read.devices.freshness.age === "error";
	const holds = holdsLead(t, held);
	if (failed)
		return {
			lead: held
				? holds
				: t(
						"devices:keys.headline.listFailed",
						"Your device list couldn't be loaded.",
					),
			rest: held
				? t(
						"devices:keys.headline.listFailedRest",
						"Your device list couldn't be loaded, so their backups can't be compared right now. The keys themselves are safe.",
					)
				: t(
						"devices:keys.headline.listFailedNone",
						"Keys and backups show once the hub answers.",
					),
		};
	return {
		lead: held
			? holds
			: t("devices:keys.headline.listLoading", "Loading your devices…"),
		rest: held
			? t(
					"devices:keys.headline.listLoadingRest",
					"Comparing them with your device list and your account backups…",
				)
			: undefined,
	};
}

function useHeadline(read: KeysRead): { lead: string; rest: ReactNode } {
	const { t } = useTranslation("devices");
	const { model, local, backups } = read;
	const of = (...categories: KeyRow["category"][]) =>
		model.rows.filter((row) => categories.includes(row.category));
	const restorable = of("restorable");
	const lost = of("lost");
	const names = (rows: KeyRow[]) => ({ 1: <FocusNames rows={rows} /> });

	if (!read.devices.loaded) return pendingListHeadline(t, read);

	if (model.keysHere === 0) {
		return {
			lead: t("keys.headline.none", "This computer has no device keys yet."),
			rest: (
				<>
					{restorable.length
						? t("keys.headline.noneRestorable", {
								count: restorable.length,
								defaultValue_one:
									"Your account holds a backup for {{count, number}} device; restore it with the device's password.",
								defaultValue_other:
									"Your account holds backups for {{count, number}} devices; restore them with each device's password.",
							})
						: backups.checkedAt === undefined
							? t(
									"keys.headline.noneChecking",
									"Checking your account for key backups…",
								)
							: t(
									"keys.headline.noneBackups",
									"Your account holds no key backups for devices in your list.",
								)}
					{lost.length ? (
						<>
							{" "}
							{lost.length === 1 ? (
								<Trans
									t={t}
									i18nKey="keys.headline.lostOne"
									defaults="<1/> has no account backup, so its keys are only on the computer that set it up."
									components={names(lost)}
								/>
							) : (
								<Trans
									t={t}
									i18nKey="keys.headline.lostMany"
									defaults="<1/> have no account backup, so their keys are only on the computers that set them up."
									components={names(lost)}
								/>
							)}
						</>
					) : null}
				</>
			),
		};
	}

	const risks = RISK_ORDER.map((kind) => ({ kind, rows: of(kind) })).filter(
		(risk) => risk.rows.length > 0,
	);

	const atRisk = storageAtRisk(local);
	const lead = atRisk
		? t("keys.headline.leadWeb", {
				count: model.keysHere,
				defaultValue_one:
					"This browser holds keys for {{count, number}} device, and may delete them.",
				defaultValue_other:
					"This browser holds keys for {{count, number}} devices, and may delete them.",
			})
		: holdsLead(t, model.keysHere);
	const quiet = risks.length === 0 && restorable.length === 0;
	return {
		lead,
		rest: (
			<>
				{risks.map((risk) => (
					<Fragment key={risk.kind}>
						<RiskSentence kind={risk.kind} rows={risk.rows} />{" "}
					</Fragment>
				))}
				{restorable.length
					? `${t("keys.headline.restorable", {
							count: restorable.length,
							defaultValue_one:
								"{{count, number}} more device can be restored from your account.",
							defaultValue_other:
								"{{count, number}} more devices can be restored from your account.",
						})} `
					: null}
				{quiet
					? model.unchecked.length
						? t(
								"keys.headline.checking",
								"Comparing them with your account backups…",
							)
						: t(
								"keys.headline.allBacked",
								"All of them are backed up to your account.",
							)
					: null}
				{atRisk
					? ` ${t(
							"keys.headline.webRisk",
							"Keep keys safely, then back up what's missing.",
						)}`
					: null}
			</>
		),
	};
}

interface SummaryWindow {
	group: KeyGroup | null;
	tone: Tone | "neutral";
	icon: LucideIcon;
	label: string;
	count: number;
	sub: string;
	title: string;
}

function behindLook(
	t: DevicesT,
	rows: readonly KeyRow[],
): { label: string; tone: Tone; icon: LucideIcon } {
	const kinds = new Set(rows.map((row) => row.category));
	const only = kinds.size === 1 ? [...kinds][0] : undefined;
	if (only === "oldpw")
		return {
			label: t("keys.summary.oldpw", "Old password on account"),
			tone: "warning",
			icon: TriangleAlert,
		};
	if (only === "hub_newer")
		return {
			label: t("keys.summary.hubNewer", "Newer on your account"),
			tone: "warning",
			icon: TriangleAlert,
		};
	if (only === "out_of_date")
		return {
			label: t("keys.state.outOfDate", "Backup out of date"),
			tone: "warning",
			icon: CloudOff,
		};
	if (only === "pending" || kinds.size === 0)
		return {
			label: t("keys.state.pending", "Upload pending"),
			tone: "info",
			icon: LoaderCircle,
		};
	return {
		label: t("keys.summary.behind", "Account copy differs"),
		tone: "info",
		icon: LoaderCircle,
	};
}

function useSummaryWindows(model: KeysModel): SummaryWindow[] {
	const { t } = useTranslation("devices");
	const sub = (rows: readonly KeyRow[], extra?: string) => {
		if (!rows.length) return t("keys.summary.none", "none right now");
		const shown = rows
			.slice(0, 3)
			.map((row) => row.name)
			.join(", ");
		const list =
			rows.length > 3
				? t("keys.summary.more", "{{names}} +{{count, number}} more", {
						names: shown,
						count: rows.length - 3,
					})
				: shown;
		return extra ? `${list} · ${extra}` : list;
	};
	const { groups } = model;
	const behind = behindLook(t, groups.behind);
	const title = (rows: readonly KeyRow[], label: string) =>
		rows.map((row) => row.name).join(", ") || label;
	const lost = groups.nokeys.some((row) => row.category === "lost");
	const labels = {
		here: t("keys.summary.here", "Keys here"),
		synced: t("keys.summary.synced", "Backed up"),
		never: t("keys.state.never", "Not backed up"),
		restorable: t("keys.summary.restorable", "Restorable"),
		nokeys: t("keys.cell.noKeys", "No keys here"),
	};
	return [
		{
			group: null,
			tone: "neutral",
			icon: KeyRound,
			label: labels.here,
			count: model.keysHere,
			sub: t("keys.summary.hereSub", {
				count: model.totalDevices,
				defaultValue_one: "of {{count, number}} device in your list",
				defaultValue_other: "of {{count, number}} devices in your list",
			}),
			title: labels.here,
		},
		{
			group: "synced",
			tone: "good",
			icon: CloudUpload,
			label: labels.synced,
			count: groups.synced.length,
			sub: sub(groups.synced),
			title: title(groups.synced, labels.synced),
		},
		{
			group: "behind",
			tone: behind.tone,
			icon: behind.icon,
			label: behind.label,
			count: groups.behind.length,
			sub: sub(groups.behind),
			title: title(groups.behind, behind.label),
		},
		{
			group: "never",
			tone: "warning",
			icon: CloudOff,
			label: labels.never,
			count: groups.never.length,
			sub: sub(
				groups.never,
				groups.never.length
					? t("keys.summary.onlyHere", "only here")
					: undefined,
			),
			title: title(groups.never, labels.never),
		},
		{
			group: "restorable",
			tone: "info",
			icon: RotateCcw,
			label: labels.restorable,
			count: groups.restorable.length,
			sub: sub(groups.restorable),
			title: title(groups.restorable, labels.restorable),
		},
		{
			group: "nokeys",
			tone: lost ? "warning" : "unknown",
			icon: CircleDashed,
			label: labels.nokeys,
			count: groups.nokeys.length,
			sub: sub(groups.nokeys),
			title: title(groups.nokeys, labels.nokeys),
		},
	];
}

/** SPEC §5.9 summary: six joined windows over the device list; pressing one filters the table. */
function KeysSummary({
	windows,
	model,
	pressed,
	onSelect,
	onLocalOnly,
}: Readonly<{
	windows: readonly SummaryWindow[];
	model: KeysModel;
	pressed: KeyGroup | null;
	onSelect(group: KeyGroup | null): void;
	onLocalOnly(): void;
}>) {
	const { t } = useTranslation("devices");
	const revoked = model.localOnly.filter((row) => row.reason === "revoked");
	return (
		<div className="@container/annun flex min-w-0 flex-col gap-2">
			<fieldset
				aria-label={t(
					"keys.summary.label",
					"Keys summary. Select a window to filter the device list.",
				)}
				className="m-0 grid min-w-0 grid-cols-6 gap-px overflow-hidden rounded-lg border border-border bg-hairline p-0 @max-[900px]/annun:grid-cols-3 @max-[480px]/annun:grid-cols-2"
			>
				{windows.map((window) => {
					const Icon = window.icon;
					const lit = window.count > 0;
					const isPressed = window.group !== null && pressed === window.group;
					const ink = !lit
						? "text-muted-foreground"
						: window.tone === "neutral"
							? "text-foreground"
							: TONE_TEXT[window.tone];
					return (
						<button
							key={window.group ?? "here"}
							type="button"
							data-window={window.group ?? "here"}
							data-tone={window.tone}
							data-lit={lit}
							aria-pressed={isPressed}
							title={window.title}
							onClick={() =>
								onSelect(
									window.group === null || isPressed ? null : window.group,
								)
							}
							className={cx(
								"grid min-w-0 cursor-pointer grid-cols-[auto_minmax(0,1fr)] grid-rows-[auto_auto] items-center gap-x-2.5 border-0 bg-card px-3.5 py-2.5 text-left text-muted-foreground hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
								isPressed && "outline-2 -outline-offset-2 outline-foreground",
							)}
						>
							<span
								className={cx(
									"row-span-2 font-mono text-[22px] leading-6.5 font-medium tabular-nums",
									ink,
								)}
							>
								{window.count}
							</span>
							<span
								className={cx(
									"inline-flex min-w-0 items-center gap-1.25 truncate text-ui font-medium",
									lit && ink,
								)}
							>
								<Icon aria-hidden className="size-3.5 shrink-0" />
								<span className="truncate">{window.label}</span>
							</span>
							<span className="truncate text-xs text-muted-foreground">
								{window.sub}
							</span>
						</button>
					);
				})}
			</fieldset>
			<p className="flex max-w-[110ch] flex-wrap gap-x-1 text-xs text-muted-foreground">
				<span>
					{t("keys.summary.caption", {
						count: model.totalDevices,
						defaultValue_one:
							"Counts the {{count, number}} device in your list.",
						defaultValue_other:
							"Counts the {{count, number}} devices in your list.",
					})}
				</span>
				<span>
					<SummaryLeftovers revoked={revoked} onLocalOnly={onLocalOnly} />
				</span>
				<span>
					{t(
						"keys.summary.filterHint",
						"Select a window to filter the device list.",
					)}
				</span>
			</p>
		</div>
	);
}

/** Where the keys that are not in the table went: the caption's middle sentence. */
function SummaryLeftovers({
	revoked,
	onLocalOnly,
}: Readonly<{
	revoked: readonly { name: string }[];
	onLocalOnly(): void;
}>) {
	const { t } = useTranslation("devices");
	const names = useNames();
	const localOnly = (
		<DvButton variant="link" size="xs" onClick={onLocalOnly}>
			{t("keys.localOnly.title", "Local-only keys")}
		</DvButton>
	);
	return (
		<>
			{revoked.length === 1 ? (
				<Trans
					t={t}
					i18nKey="keys.summary.revokedOne"
					defaults="{{devices}} is revoked; its keys are under <1/>."
					values={{ devices: revoked[0]?.name }}
					components={{ 1: localOnly }}
				/>
			) : revoked.length ? (
				<Trans
					t={t}
					i18nKey="keys.summary.revokedMany"
					defaults="{{devices}} are revoked; their keys are under <1/>."
					values={{
						devices: names(revoked.slice(0, 3).map((row) => row.name)),
					}}
					components={{ 1: localOnly }}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="keys.summary.leftovers"
					defaults="Keys for revoked devices and unapproved requests are under <1/>."
					components={{ 1: localOnly }}
				/>
			)}
		</>
	);
}

/** Why "Unlock several…" is off: nothing left to unlock, no keys here, or the device list is not there yet. */
function useUnlockGate(read: KeysRead, anyLocked: boolean) {
	const { t } = useTranslation("devices");
	if (!read.devices.loaded)
		return {
			kind: "busy" as const,
			reason: t(
				"keys.computer.listPending",
				"Available once your device list has loaded.",
			),
		};
	if (anyLocked) return null;
	return {
		kind: "nokeys" as const,
		reason: read.model.rows.some((row) => row.vault)
			? t(
					"keys.guide.new.allUnlocked",
					"Every device with keys here is unlocked.",
				)
			: t("keys.table.noKeysYet", "No keys on this computer yet."),
	};
}

/** SPEC §5.9 "This computer": storage, scope, passwords, open key sessions. */
function ThisComputer({
	read,
	flows,
}: Readonly<{ read: KeysRead; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	/* Open key sessions are this computer's own fact: they count without the device list. */
	const { lockAll, unlockedCount } = useKeyChip();
	const setting = useAccessChangePassword();
	const { scope, profile } = workspace.deps;
	const { model, local } = read;
	const withKeys = model.rows.filter((row) => row.vault);
	const open = withKeys.filter((row) => row.session?.state === "unlocked");
	const locked = withKeys.filter((row) => row.session?.state === "locked");
	const risk = storageAtRisk(local);
	const unlockGate = useUnlockGate(read, locked.length > 0);
	const lockGate = unlockedCount
		? null
		: {
				kind: "locked" as const,
				reason: t(
					"keys.computer.nothingOpen",
					"Nothing is unlocked right now.",
				),
			};
	return (
		<Block
			icon={Laptop}
			title={t("keys.computer.title", "This computer")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("keys.stamp.localShort", "stored in this app")}
				/>
			}
		>
			<KeyValueList>
				<KvRow label={t("keys.computer.storage", "Storage")}>
					<span
						className={cx(
							"inline-flex items-center gap-1",
							risk && "text-warning",
						)}
					>
						{risk ? (
							<TriangleAlert aria-hidden className="size-3.5 shrink-0" />
						) : (
							<ShieldCheck aria-hidden className="size-3.5 shrink-0" />
						)}
						{storageLabel(t, local)}
					</span>
					<KvHint>{storageHint(t, local)}</KvHint>
					<StorageResult />
				</KvRow>
				<KvRow label={t("keys.computer.storedFor", "Stored for")}>
					<Trans
						t={t}
						i18nKey="keys.computer.storedForValue"
						defaults="Your account · {{hub}} · app profile <1/>"
						values={{ hub: hostOf(scope.apiOrigin) }}
						components={{ 1: <Mono>{profile.name || scope.profileId}</Mono> }}
					/>
					<KvHint>
						{t(
							"keys.computer.storedForHint",
							"Another account, hub or app profile keeps its own separate keys. Switching shows a different set.",
						)}
					</KvHint>
				</KvRow>
				<KvRow label={t("keys.computer.passwords", "Device passwords")}>
					{t("keys.computer.passwordsValue", "One per device on this computer")}
					<KvHint>
						{t(
							"keys.computer.passwordsHint",
							"It isn't your account password. The password never leaves this computer and the hub never sees it. If you forget it, nobody can recover it.",
						)}
					</KvHint>
				</KvRow>
				<KvRow label={t("keys.computer.unlocked", "Unlocked now")}>
					{unlockedCount ? (
						<>
							{t("keys.computer.unlockedCount", {
								count: unlockedCount,
								defaultValue_one: "{{count, number}} device",
								defaultValue_other: "{{count, number}} devices",
							})}
							{open.length ? (
								<>
									{" · "}
									<MonoNames names={open.slice(0, 3).map((row) => row.name)} />
									{open.length > 3 ? " …" : null}
								</>
							) : null}
						</>
					) : (
						t("keys.computer.unlockedNone", "None")
					)}
					<KvHint>
						{t(
							"keys.computer.unlockedHint",
							"Unlocked devices lock after 30 min unused. Locking clears logs and decrypted history from this window.",
						)}
					</KvHint>
				</KvRow>
			</KeyValueList>
			<div className="flex flex-wrap items-start gap-2">
				<GatedAction gate={unlockGate}>
					<DvButton
						size="sm"
						icon={LockOpen}
						onClick={() => flows.unlockSeveral()}
					>
						{t("keys.unlockSeveral", "Unlock several…")}
					</DvButton>
				</GatedAction>
				<GatedAction gate={lockGate}>
					<DvButton
						size="sm"
						variant="ghost"
						icon={Lock}
						onClick={() => lockAll()}
					>
						{t("keys.computer.lockAll", "Lock all")}
					</DvButton>
				</GatedAction>
			</div>
			<div className="flex flex-col gap-1 border-t border-hairline pt-3">
				<SwitchField
					id="keys-ask-password"
					checked={setting.ask}
					onCheckedChange={setting.setAsk}
				>
					{t(
						"keys.computer.askPassword",
						"Ask for my password again for access changes",
					)}
				</SwitchField>
				<p className="pl-10.5 text-xs text-muted-foreground">
					{t(
						"keys.computer.askPasswordHint",
						"While a device is unlocked, sharing and reader changes are signed with its owner key. Turn this on to type the device password for each change.",
					)}
				</p>
			</div>
		</Block>
	);
}

/** `focus=<device>` on a device without keys here: say so and offer the ways to get them. */
function FocusBanner({
	row,
	flows,
}: Readonly<{ row: KeyRow; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const hub = row.hubRevision;
	return (
		<Banner
			tone="info"
			icon={KeyRound}
			title={
				<Trans
					t={t}
					i18nKey="keys.focus.title"
					defaults="This computer has no keys for <1/>."
					components={{ 1: <Mono>{row.name}</Mono> }}
				/>
			}
			actions={
				<>
					{hub ? (
						<DvButton
							size="sm"
							icon={RotateCcw}
							onClick={() =>
								flows.open({ kind: "restore", deviceId: row.deviceId })
							}
						>
							{t("keys.menu.restore", "Restore from account backup…")}
						</DvButton>
					) : null}
					<DvButton
						size="sm"
						icon={FileUp}
						onClick={() => flows.open({ kind: "import" })}
					>
						{t("keys.menu.import", "Import backup file…")}
					</DvButton>
					{row.relationship === "owner" ? null : (
						<DvButton size="sm" variant="ghost" asChild>
							<a
								{...link({
									screen: "access",
									tab: "shared",
									action: "request",
								})}
							>
								{t("keys.menu.request", "Request access again")}
							</a>
						</DvButton>
					)}
				</>
			}
		>
			{t(
				"keys.focus.text",
				"Keys are kept per app profile on each computer, so they may still be on another computer or profile.",
			)}{" "}
			{hub
				? t(
						"keys.focus.backup",
						"Your account holds a backup (v{{version}}); restore it with the device password.",
						{ version: hub },
					)
				: hub === 0
					? t(
							"keys.focus.noBackup",
							"There's no account backup. Import its backup file, or use the computer that set it up.",
						)
					: null}
		</Banner>
	);
}

function TopResult() {
	const results = useKeyResults();
	const result = results.of(RESTORE_RESULT);
	if (!result) return null;
	return (
		<InlineResult
			tone={result.tone}
			onDismiss={() => results.dismiss(RESTORE_RESULT)}
		>
			{result.text}
		</InlineResult>
	);
}

function scrollToBlock(id: string) {
	globalThis.document?.getElementById(id)?.scrollIntoView?.({ block: "start" });
}

const FLEET_HOME: DevicesRoute = { screen: "fleet", view: "devices" };

/** `total` is undefined until the device list answered: the count is then left out, not shown as 0. */
function PageSub({ total }: Readonly<{ total: number | undefined }>) {
	const { t } = useTranslation("devices");
	const { scope, profile } = useDeviceWorkspace().deps;
	const values = { count: total ?? 0, hub: hostOf(scope.apiOrigin) };
	const components = { 1: <Mono>{profile.name || scope.profileId}</Mono> };
	if (total === undefined)
		return (
			<Trans
				t={t}
				i18nKey="keys.subPending"
				defaults="Keys stored in this app for your account · {{hub}} · app profile <1/>"
				values={values}
				components={components}
			/>
		);
	return total === 1 ? (
		<Trans
			t={t}
			i18nKey="keys.subOne"
			defaults="{{count, number}} device in your list · keys stored in this app for your account · {{hub}} · app profile <1/>"
			values={values}
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="keys.subMany"
			defaults="{{count, number}} devices in your list · keys stored in this app for your account · {{hub}} · app profile <1/>"
			values={values}
			components={components}
		/>
	);
}

/** N9 Keys & recovery (SPEC §5.9, IA §6.2 N9): which devices this computer can manage and whether those keys are safe. */
export function KeysScreen({ route }: Readonly<ScreenProps>) {
	const { t } = useTranslation("devices");
	const { navigate, href } = useDevicesRoute();
	const read = useKeysModel();
	const files = useKeyFileLog();
	const { api: flows, sheets } = useKeyFlows(read, files);
	const keysRoute = route.screen === "keys" ? route : KEYS_HOME;
	const { guide, focusDeviceId } = keysRoute;
	const [filter, setFilter] = useState<KeyGroup | null>(null);
	const [open, setOpen] = useState({
		newComputer: guide === "new-computer",
		forgot: guide === "forgot-password",
	});
	const [forgotDevice, setForgotDevice] = useState<string | undefined>(
		guide === "forgot-password" ? focusDeviceId : undefined,
	);
	const { model, local } = read;
	const windows = useSummaryWindows(model);
	const headline = useHeadline(read);

	useEffect(() => {
		if (!guide) return;
		setOpen((current) =>
			guide === "new-computer"
				? { ...current, newComputer: true }
				: { ...current, forgot: true },
		);
		scrollToBlock(`guide-${guide}`);
	}, [guide]);

	const toggle = useCallback(
		(which: "newComputer" | "forgot") => {
			const next = !open[which];
			setOpen((current) => ({ ...current, [which]: next }));
			navigate(
				{
					screen: "keys",
					...(next
						? {
								guide:
									which === "newComputer" ? "new-computer" : "forgot-password",
							}
						: {}),
					...(focusDeviceId ? { focusDeviceId } : {}),
				},
				{ replace: true },
			);
		},
		[open, navigate, focusDeviceId],
	);

	const selectGroup = (group: KeyGroup | null) => {
		setFilter(group);
		if (group) scrollToBlock("keys-table");
	};

	const focused = focusDeviceId
		? model.rows.find((row) => row.deviceId === focusDeviceId)
		: undefined;
	const newComputer =
		read.devices.loaded &&
		model.keysHere === 0 &&
		model.rows.some(
			(row) => row.category === "restorable" || row.category === "lost",
		);
	const guideNew = (
		<GuideNewComputer
			read={read}
			prominent={newComputer}
			open={open.newComputer}
			onToggle={() => toggle("newComputer")}
			flows={flows}
		/>
	);
	const guideForgot = (
		<GuideForgotPassword
			read={read}
			open={open.forgot}
			onToggle={() => toggle("forgot")}
			deviceId={forgotDevice}
			onDevice={setForgotDevice}
			flows={flows}
		/>
	);
	const side = open.forgot || (!newComputer && open.newComputer);
	const pair =
		"grid grid-cols-2 items-start gap-6 @max-[900px]/devices:grid-cols-1";
	const groupLabel = (group: KeyGroup) =>
		windows.find((window) => window.group === group)?.label ?? "";

	return (
		<div data-screen="keys" className="flex min-w-0 flex-col gap-6">
			<PersistenceNotice
				local={local}
				keysHere={model.keysHere}
				onShowBackups={() => scrollToBlock("keys-table")}
			/>
			{focused &&
			!focused.vault &&
			focused.relationship !== "cloud_approval" ? (
				<FocusBanner row={focused} flows={flows} />
			) : null}
			<PageHeader
				crumbs={[
					{
						label: t("keys.crumb.devices", "Devices"),
						href: href(FLEET_HOME),
						onNavigate: () => navigate(FLEET_HOME),
					},
					{ label: t("keys.title", "Keys & recovery") },
				]}
				title={t("keys.title", "Keys & recovery")}
				sub={
					<PageSub
						total={read.devices.loaded ? model.totalDevices : undefined}
					/>
				}
				actions={
					<>
						<DvButton
							icon={FileUp}
							onClick={() => flows.open({ kind: "import" })}
						>
							{t("keys.header.import", "Import backup files…")}
						</DvButton>
						<DvButton
							icon={RotateCcw}
							onClick={() => flows.open({ kind: "restore" })}
						>
							{t("keys.header.restore", "Restore keys…")}
						</DvButton>
					</>
				}
			/>
			<TopResult />
			<Headline lead={headline.lead} rest={headline.rest} />
			{read.devices.loaded ? (
				<KeysSummary
					windows={windows}
					model={model}
					pressed={filter}
					onSelect={selectGroup}
					onLocalOnly={() => scrollToBlock("local-only")}
				/>
			) : null}
			{newComputer ? guideNew : null}
			<div className={pair}>
				<ThisComputer read={read} flows={flows} />
				<AccountBackupPanel read={read} />
			</div>
			<KeysTable
				read={read}
				files={files}
				filter={filter}
				onFilter={setFilter}
				focusDeviceId={focused ? focusDeviceId : undefined}
				flows={flows}
				groupLabel={groupLabel}
			/>
			<LocalOnlyKeys read={read} flows={flows} />
			{newComputer ? (
				guideForgot
			) : side ? (
				<>
					{guideNew}
					{guideForgot}
				</>
			) : (
				<div className={pair}>
					{guideNew}
					{guideForgot}
				</div>
			)}
			{sheets}
		</div>
	);
}
