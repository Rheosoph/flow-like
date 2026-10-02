"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Check,
	CircleCheck,
	Cloud,
	CloudOff,
	CloudUpload,
	Copy,
	Download,
	Ellipsis,
	FileUp,
	FolderKey,
	Funnel,
	History,
	KeyRound,
	LoaderCircle,
	Lock,
	LockOpen,
	type LucideIcon,
	RefreshCw,
	RotateCcw,
	ShieldCheck,
	Trash2,
	TriangleAlert,
	UserPlus,
	X,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import { groupFingerprint } from "../../../../lib/device-management/fingerprint";
import type { ActionId } from "../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { identityName } from "../access/person-name";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { RequestedActual } from "../primitives/requested-actual";
import { FilterChip } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import { KeyChip, StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { useOverlay } from "../workspace/overlay-store";
import { useKeyChip } from "../workspace/use-keys";
import type { KeyFlowsApi } from "./key-flows";
import {
	DayOf,
	KEYS_TABLE_CLASS,
	accountVersionLabel,
	dayLabel,
	keyChipOf,
	keyKindLabel,
} from "./key-parts";
import {
	type KeyFileLog,
	type KeyResults,
	rowScope,
	useKeyResults,
} from "./key-store";
import {
	type KeyGroup,
	type KeyRow,
	type LocalOnlyRow,
	keyGroupOf,
	localRevision,
	rowsWithKeys,
} from "./keys-model";
import { useKeyGate } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

const FIRST_PAGE = 25;
const NEXT_PAGE = 50;
const LOCAL_ONLY_CAP = 8;
const DAY_MS = 86_400_000;

const MENU_CONTENT =
	"min-w-56 border-border-strong bg-popover shadow-none backdrop-blur-none";
const MENU_ITEM =
	"items-start text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";

const KEYS_COLS = ["16%", "12.5%", "17%", "21%", "12.5%", "16%", "5%"] as const;
const LOCAL_COLS = ["22%", "42%", "14%", "22%"] as const;

/** BG21: the fingerprint as the device prints it, four groups of four, case kept. */
export function Fingerprint({ value }: Readonly<{ value: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const grouped = groupFingerprint(value);
	return (
		<span className="inline-flex max-w-full min-w-0 items-center gap-0.5 align-middle">
			<span
				data-fingerprint=""
				title={t(
					"keys.fingerprint.title",
					"Compare with what `flow-like-standalone status` prints on the device.",
				)}
				className="min-w-0 truncate rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px font-mono text-xs whitespace-nowrap text-ink-2"
			>
				{grouped}
			</span>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={
					copied
						? t("keys.fingerprint.copied", "Copied")
						: t("keys.fingerprint.copy", "Copy fingerprint")
				}
				onClick={() => void copy(grouped)}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

/** A consent-only device: what the viewer does for it, or that it was revoked. */
function CloudApprovalSub({ device }: Readonly<{ device: KeyRow["row"] }>) {
	const { t } = useTranslation("devices");
	const revoked = device.status === "revoked";
	return (
		<>
			{revoked
				? t("keys.relationship.revoked", "revoked")
				: t(
						"keys.relationship.cloudApprovalSub",
						"you approve or pay for its cloud access",
					)}
			{revoked && device.revoked_at ? (
				<>
					{" "}
					<DayOf atS={device.revoked_at} />
				</>
			) : null}
		</>
	);
}

/** Names the owner once the directory knows them. */
function SharedBy({ ownerId }: Readonly<{ ownerId: string }>) {
	const { t } = useTranslation("devices");
	const name = identityName(useUserIdentity(ownerId), ownerId);
	return name
		? t("keys.relationship.sharedBy", "Shared by {{name}}", { name })
		: t("keys.relationship.shared", "Shared with you");
}

function RelationshipCell({ row }: Readonly<{ row: KeyRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (row.relationship === "owner")
		return (
			<>
				{t("keys.relationship.owner", "Yours")}
				<CellSub>
					{t("keys.relationship.setUp", "set up")}{" "}
					<DayOf atS={row.row.registered_at} />
				</CellSub>
			</>
		);
	if (row.relationship === "cloud_approval")
		return (
			<>
				{t("keys.relationship.cloudApproval", "Cloud approvals only")}
				<CellSub>
					<CloudApprovalSub device={row.row} />
				</CellSub>
			</>
		);
	const endsAt = row.row.access_expires_at;
	return (
		<>
			{row.relationship === "shared" ? (
				<SharedBy ownerId={row.row.owner_id} />
			) : (
				t("keys.relationship.unknown", "Shared or cloud approvals")
			)}
			{endsAt ? (
				<CellSub title={time.abs(endsAt)}>
					{endsAt > time.nowS
						? t("keys.relationship.ends", "ends {{in}}", {
								in: time.ago(endsAt),
							})
						: t("keys.relationship.ended", "ended {{ago}}", {
								ago: time.ago(endsAt),
							})}
				</CellSub>
			) : null}
		</>
	);
}

function KeysCell({ row }: Readonly<{ row: KeyRow }>) {
	const { t } = useTranslation("devices");
	if (!row.vault) {
		if (row.category === "restorable")
			return (
				<>
					{t("keys.cell.noKeys", "No keys here")}
					<CellSub>
						{t(
							"keys.cell.restorable",
							"Restore them from your account backup (v{{version}}).",
							{ version: row.hubRevision ?? 0 },
						)}
					</CellSub>
				</>
			);
		if (row.category === "lost")
			return (
				<>
					<span className="text-warning">
						{t("keys.cell.noKeys", "No keys here")}
					</span>
					<CellSub>{t("keys.cell.lost", "No account backup either.")}</CellSub>
				</>
			);
		return (
			<>
				<span className="text-muted-foreground">
					{t("keys.cell.noKeys", "No keys here")}
				</span>
				<CellSub>
					{row.relationship === "cloud_approval"
						? t(
								"keys.cell.consentOnly",
								"You only approve its cloud access, so there are no keys to hold.",
							)
						: t("keys.cell.neverHeld", "Never held on this computer.")}
				</CellSub>
			</>
		);
	}
	return (
		<>
			{keyKindLabel(t, row.vault.role)}
			<CellSub>
				<KeyChip {...keyChipOf(row)} />
			</CellSub>
			{row.vault.requiresFreshEndpoint ? (
				<CellSub className="flex items-start gap-1">
					<History aria-hidden className="mt-0.5 size-3 shrink-0" />
					{t(
						"keys.cell.restored",
						"Restored · the first unlock refreshes this computer's identity for shared live metrics",
					)}
				</CellSub>
			) : null}
		</>
	);
}

function CellAction({
	action,
	deviceId,
	icon,
	busy,
	onClick,
	children,
}: Readonly<{
	action: ActionId;
	deviceId: string;
	icon: LucideIcon;
	busy?: boolean;
	onClick(): void;
	children: ReactNode;
}>) {
	const gate = useKeyGate(action, deviceId);
	return (
		<div className="mt-1.5">
			<GatedAction gate={gate}>
				<DvButton size="sm" icon={icon} busy={busy} onClick={onClick}>
					{children}
				</DvButton>
			</GatedAction>
		</div>
	);
}

function BackupCell({
	row,
	flows,
}: Readonly<{ row: KeyRow; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hub = row.hubRevision ?? 0;
	const local = localRevision(row);
	const { deviceId } = row;
	switch (row.category) {
		case "synced":
			return (
				<>
					<StatusChip
						tone="outline"
						icon={CloudUpload}
						title={t(
							"keys.backup.syncedTitle",
							"The hub stores it encrypted. You still need the device password to open it.",
						)}
					>
						{t("keys.state.synced", "Backed up (v{{version}})", {
							version: hub,
						})}
					</StatusChip>
					<CellSub
						title={row.hubSavedAt ? time.abs(row.hubSavedAt) : undefined}
					>
						{t("keys.backup.synced", "matches this computer")}
						{row.hubSavedAt ? (
							<>
								{" · "}
								<span className="whitespace-nowrap">
									{t("keys.backup.savedOn", "saved {{date}}", {
										date: dayLabel(row.hubSavedAt, time.now, time.locale),
									})}
								</span>
							</>
						) : null}
					</CellSub>
				</>
			);
		case "pending":
			return (
				<>
					<RequestedActual
						versions
						requested={t("keys.backup.here", "This computer v{{version}}", {
							version: local,
						})}
						actual={accountVersionLabel(t, row.hubRevision)}
						label={
							hub
								? t(
										"keys.backup.pendingAria",
										"This computer has version {{local}}; your account has version {{hub}}",
										{ local, hub },
									)
								: t(
										"keys.backup.pendingFirstAria",
										"This computer has version {{local}}; it hasn't reached your account yet",
										{ local },
									)
						}
						tone="info"
						icon={LoaderCircle}
						sub={t("keys.state.pending", "Upload pending")}
					/>
					<CellAction
						action="account_backup_save"
						deviceId={deviceId}
						icon={RefreshCw}
						busy={flows.busy(deviceId)}
						onClick={() => flows.retryUpload(row)}
					>
						{t("keys.backup.retry", "Retry upload")}
					</CellAction>
				</>
			);
		case "oldpw":
			return (
				<>
					<RequestedActual
						versions
						requested={t("keys.backup.newPassword", "New password here")}
						actual={t(
							"keys.backup.accountOld",
							"account v{{version}} · old password",
							{ version: hub },
						)}
						label={t(
							"keys.backup.oldpwAria",
							"Password changed on this computer; account backup version {{version}} still opens with the old password",
							{ version: hub },
						)}
						tone="warning"
						icon={TriangleAlert}
						sub={t(
							"keys.backup.oldpwSub",
							"The account copy still opens with the old password.",
						)}
					/>
					<CellAction
						action="account_backup_save"
						deviceId={deviceId}
						icon={CloudUpload}
						onClick={() =>
							flows.open({ kind: "backup", deviceId, mode: "update" })
						}
					>
						{t("keys.backup.update", "Update account backup")}
					</CellAction>
				</>
			);
		case "out_of_date":
			return (
				<>
					<StatusChip tone="warning" icon={CloudOff}>
						{t("keys.state.outOfDate", "Backup out of date")}
					</StatusChip>
					<CellSub>
						{t(
							"keys.backup.outOfDateSub",
							"The keys here changed after v{{version}} was saved.",
							{ version: hub },
						)}
					</CellSub>
					<CellAction
						action="account_backup_save"
						deviceId={deviceId}
						icon={CloudUpload}
						onClick={() =>
							flows.open({ kind: "backup", deviceId, mode: "update" })
						}
					>
						{t("keys.backup.update", "Update account backup")}
					</CellAction>
				</>
			);
		case "hub_newer":
			return (
				<>
					<RequestedActual
						versions
						requested={t("keys.backup.here", "This computer v{{version}}", {
							version: local,
						})}
						actual={t("keys.backup.account", "account v{{version}}", {
							version: hub,
						})}
						label={t(
							"keys.backup.hubNewerAria",
							"This computer has version {{local}}; your account has the newer version {{hub}}",
							{ local, hub },
						)}
						tone="warning"
						icon={TriangleAlert}
						sub={t(
							"keys.backup.hubNewerSub",
							"Another computer saved a newer backup. Check it to adopt its version.",
						)}
					/>
					<CellAction
						action="account_backup_save"
						deviceId={deviceId}
						icon={ShieldCheck}
						onClick={() =>
							flows.open({ kind: "backup", deviceId, mode: "check" })
						}
					>
						{t("keys.backup.check", "Check account backup…")}
					</CellAction>
				</>
			);
		case "never":
			return (
				<>
					<StatusChip
						tone="warning"
						icon={CloudOff}
						title={t(
							"keys.backup.neverTitle",
							"The keys exist only on this computer.",
						)}
					>
						{t("keys.state.never", "Not backed up")}
					</StatusChip>
					<CellSub>
						{t("keys.backup.neverSub", "Only on this computer")}
					</CellSub>
					<CellAction
						action="account_backup_save"
						deviceId={deviceId}
						icon={CloudUpload}
						onClick={() =>
							flows.open({ kind: "backup", deviceId, mode: "save" })
						}
					>
						{t("keys.backup.backUp", "Back up to account")}
					</CellAction>
				</>
			);
		case "restorable":
			return (
				<>
					<StatusChip tone="outline" icon={Cloud}>
						{t("keys.state.restorable", "On your account (v{{version}})", {
							version: hub,
						})}
					</StatusChip>
					<CellSub>
						{t("keys.backup.restorableSub", "ready to restore")}
					</CellSub>
					<CellAction
						action="account_backup_restore"
						deviceId={deviceId}
						icon={RotateCcw}
						onClick={() => flows.open({ kind: "restore", deviceId })}
					>
						{t("keys.backup.restore", "Restore…")}
					</CellAction>
				</>
			);
		case "lost":
			return (
				<>
					<StatusChip tone="warning" icon={CloudOff}>
						{t("keys.state.lost", "None on your account")}
					</StatusChip>
					<CellSub>
						{t(
							"keys.backup.lostSub",
							"Import its backup file, or use the computer that set it up.",
						)}
					</CellSub>
					<CellAction
						action="import_key_file"
						deviceId={deviceId}
						icon={FileUp}
						onClick={() => flows.open({ kind: "import" })}
					>
						{t("keys.backup.import", "Import backup file…")}
					</CellAction>
				</>
			);
		case "unchecked":
			return (
				<>
					<span className="text-muted-foreground">
						{t("keys.state.unchecked", "Not checked yet")}
					</span>
					<CellSub>
						{t(
							"keys.backup.uncheckedSub",
							"Check backups to compare with your account.",
						)}
					</CellSub>
				</>
			);
		default:
			return (
				<>
					<span className="text-muted-foreground">–</span>
					<CellSub>{t("keys.state.nokeys", "No keys to back up")}</CellSub>
				</>
			);
	}
}

function FileCell({
	row,
	files,
}: Readonly<{ row: KeyRow; files: KeyFileLog }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!row.vault) return <span className="text-muted-foreground">–</span>;
	const record = files[row.deviceId];
	const savedAt = record?.savedAt;
	const atRisk = row.category === "never";
	if (savedAt === undefined)
		return (
			<>
				<span className={atRisk ? "text-warning" : "text-muted-foreground"}>
					{t("keys.file.none", "Not saved here")}
				</span>
				<CellSub>
					{atRisk
						? t(
								"keys.file.noneRisk",
								"A file is a second copy you keep yourself.",
							)
						: t("keys.file.noneOptional", "optional second copy")}
				</CellSub>
			</>
		);
	const savedAtS = Math.floor(savedAt / 1000);
	const old =
		record?.passwordChangedAt !== undefined &&
		record.passwordChangedAt > savedAt;
	return (
		<>
			{time.now - savedAt < DAY_MS ? (
				<time title={time.abs(savedAtS)}>{time.ago(savedAtS, "long")}</time>
			) : (
				<DayOf atS={savedAtS} />
			)}
			<CellSub className={old ? "text-warning" : undefined}>
				{old
					? t("keys.file.oldPassword", "opens with the old password")
					: t("keys.file.savedHere", "saved from this computer")}
			</CellSub>
		</>
	);
}

function IdentityCell({ row }: Readonly<{ row: KeyRow }>) {
	const { t } = useTranslation("devices");
	if (row.relationship === "cloud_approval")
		return <span className="text-muted-foreground">–</span>;
	if (!row.vault)
		return (
			<>
				<span className="text-muted-foreground">
					{t("keys.identity.notYet", "Not yet")}
				</span>
				<CellSub>
					{t("keys.identity.firstUnlock", "checked when you first unlock")}
				</CellSub>
			</>
		);
	const { identityPinnedAt, identityFingerprint } = row.vault;
	return (
		<>
			{identityPinnedAt === undefined ? (
				t("keys.identity.onFirstUnlock", "On first unlock")
			) : (
				<DayOf atS={Math.floor(identityPinnedAt / 1000)} />
			)}
			{identityFingerprint ? (
				<CellSub>
					<Fingerprint value={identityFingerprint} />
				</CellSub>
			) : null}
		</>
	);
}

interface MenuEntry {
	id: string;
	label: string;
	icon: LucideIcon;
	note?: ReactNode;
	disabled?: boolean;
	danger?: boolean;
	separated?: boolean;
	/** The gate of a key action: while it fails the item is off and says why (R7). */
	action?: ActionId;
	run?(): void;
}

function menuEntries(
	t: DevicesT,
	row: KeyRow,
	flows: KeyFlowsApi,
	session: {
		lock(): void;
		unlock(): void;
		open(): void;
		requestAccess(): void;
	},
): MenuEntry[] {
	const { deviceId } = row;
	const entries: MenuEntry[] = [
		{
			id: "open",
			label: t("keys.menu.open", "Open device"),
			icon: ArrowRight,
			run: session.open,
		},
	];
	if (row.relationship === "cloud_approval")
		return [
			...entries,
			{
				id: "none",
				label: t("keys.menu.none", "No keys to manage"),
				note: t("keys.menu.noneNote", "You only approve its cloud access."),
				icon: KeyRound,
				disabled: true,
			},
		];
	if (!row.vault) {
		if (row.category === "restorable")
			entries.push({
				id: "restore",
				label: t("keys.menu.restore", "Restore from account backup…"),
				icon: RotateCcw,
				action: "account_backup_restore",
				run: () => flows.open({ kind: "restore", deviceId }),
			});
		entries.push({
			id: "import",
			label: t("keys.menu.import", "Import backup file…"),
			icon: FileUp,
			action: "import_key_file",
			run: () => flows.open({ kind: "import" }),
		});
		if (row.relationship !== "owner")
			entries.push({
				id: "request",
				label: t("keys.menu.request", "Request access again"),
				icon: UserPlus,
				run: session.requestAccess,
			});
		return entries;
	}
	const open = row.session?.state === "unlocked";
	entries.push(
		open
			? {
					id: "lock",
					label: t("keys.menu.lock", "Lock"),
					icon: Lock,
					run: session.lock,
				}
			: {
					id: "unlock",
					label: t("keys.menu.unlock", "Unlock…"),
					icon: LockOpen,
					run: session.unlock,
				},
	);
	const backup: Partial<Record<KeyRow["category"], MenuEntry>> = {
		never: {
			id: "backup",
			label: t("keys.backup.backUp", "Back up to account"),
			icon: CloudUpload,
			action: "account_backup_save",
			run: () => flows.open({ kind: "backup", deviceId, mode: "save" }),
		},
		pending: {
			id: "backup",
			label: t("keys.backup.retry", "Retry upload"),
			icon: RefreshCw,
			action: "account_backup_save",
			run: () => flows.retryUpload(row),
		},
		synced: {
			id: "backup",
			label: t("keys.backup.update", "Update account backup"),
			icon: CloudUpload,
			disabled: true,
			note: t(
				"keys.menu.updateSame",
				"Your account already has v{{version}}, the same as this computer.",
				{ version: row.hubRevision ?? 0 },
			),
		},
	};
	entries.push(
		backup[row.category] ?? {
			id: "backup",
			label: t("keys.backup.update", "Update account backup"),
			icon: CloudUpload,
			action: "account_backup_save",
			run: () => flows.open({ kind: "backup", deviceId, mode: "update" }),
		},
		{
			id: "check",
			label: t("keys.menu.check", "Check account backup…"),
			icon: ShieldCheck,
			action: "account_backup_save",
			run: () => flows.open({ kind: "backup", deviceId, mode: "check" }),
		},
		{
			id: "download",
			label: t("keys.menu.download", "Download key backup file…"),
			icon: Download,
			action: "download_key_file",
			run: () => flows.open({ kind: "download", deviceId }),
		},
		{
			id: "password",
			label: t("keys.menu.password", "Change device password…"),
			icon: KeyRound,
			action: "change_device_password",
			run: () => flows.open({ kind: "password", deviceId }),
		},
		{
			id: "delete",
			label: t("keys.menu.delete", "Delete keys from this computer…"),
			icon: Trash2,
			danger: true,
			separated: true,
			action: "delete_local_keys",
			run: () => flows.deleteKeys(row),
		},
	);
	return entries;
}

function MenuItem({ entry }: Readonly<{ entry: MenuEntry }>) {
	const Icon = entry.icon;
	return (
		<DropdownMenuItem
			disabled={entry.disabled}
			data-menu-item={entry.id}
			className={cx(
				MENU_ITEM,
				entry.danger &&
					"text-critical focus:text-critical [&_svg]:text-critical!",
			)}
			onSelect={() => entry.run?.()}
		>
			<Icon aria-hidden className="mt-0.5 size-4 shrink-0" />
			<span className="min-w-0">
				{entry.label}
				{entry.note ? (
					<span className="block text-xs text-ink-2">{entry.note}</span>
				) : null}
			</span>
		</DropdownMenuItem>
	);
}

/** A key action in the menu follows the same gate as its button in the table and on the device's Keys tab. */
function GatedMenuItem({
	entry,
	action,
	deviceId,
}: Readonly<{ entry: MenuEntry; action: ActionId; deviceId: string }>) {
	const gate = useKeyGate(action, deviceId);
	return (
		<MenuItem
			entry={gate ? { ...entry, disabled: true, note: gate.reason } : entry}
		/>
	);
}

function RowMenu({
	row,
	flows,
}: Readonly<{ row: KeyRow; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const { lock } = useKeyChip();
	const overlay = useOverlay();
	const { navigate } = useDevicesRoute();
	const { deviceId } = row;
	const entries = menuEntries(t, row, flows, {
		lock: () => lock(deviceId),
		unlock: () => overlay.openUnlock(deviceId),
		open: () => navigate({ screen: "device", deviceId, tab: "keys" }),
		requestAccess: () =>
			navigate({ screen: "access", tab: "shared", action: "request" }),
	});
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton
					variant="ghost"
					size="xs"
					iconOnly
					icon={Ellipsis}
					aria-label={t("keys.menu.more", "More for {{device}}", {
						device: row.name,
					})}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className={MENU_CONTENT}>
				{entries.map((entry) => (
					<div key={entry.id}>
						{entry.separated ? (
							<DropdownMenuSeparator className="bg-hairline" />
						) : null}
						{entry.action && !entry.disabled ? (
							<GatedMenuItem
								entry={entry}
								action={entry.action}
								deviceId={deviceId}
							/>
						) : (
							<MenuItem entry={entry} />
						)}
					</div>
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function ResultRow({
	scope,
	results,
	colSpan,
}: Readonly<{ scope: string; results: KeyResults; colSpan: number }>) {
	const result = results.of(scope);
	if (!result) return null;
	return (
		<tr data-confirm-row="" className="hover:bg-transparent">
			<td colSpan={colSpan} className="px-4 pt-0 pb-2.5 @max-[900px]/tbl:p-0">
				<InlineResult
					tone={result.tone}
					onDismiss={() => results.dismiss(scope)}
				>
					{result.text}
				</InlineResult>
			</td>
		</tr>
	);
}

function DeviceRowView({
	row,
	files,
	flows,
	results,
	focused,
}: Readonly<{
	row: KeyRow;
	files: KeyFileLog;
	flows: KeyFlowsApi;
	results: KeyResults;
	focused: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const ref = useRef<HTMLTableRowElement>(null);
	useEffect(() => {
		if (focused) ref.current?.scrollIntoView?.({ block: "center" });
	}, [focused]);
	return (
		<>
			<Tr
				ref={ref}
				selected={focused}
				dim={row.category === "nokeys"}
				data-device={row.deviceId}
				data-category={row.category}
				className="scroll-mt-16"
			>
				<Td label={t("keys.col.device", "Device")} kind="name">
					<a
						{...link({ screen: "device", deviceId: row.deviceId, tab: "keys" })}
						title={row.name}
						className={cx(
							"block truncate font-mono text-[12.5px] font-semibold no-underline hover:underline",
							row.category === "nokeys"
								? "text-muted-foreground"
								: "text-foreground",
						)}
					>
						{row.name}
					</a>
					<CellSub>
						<IdRef
							id={row.deviceId}
							copyLabel={t("keys.col.copyId", "Copy device ID")}
						/>
					</CellSub>
				</Td>
				<Td label={t("keys.col.relationship", "Relationship")}>
					<RelationshipCell row={row} />
				</Td>
				<Td label={t("keys.col.keys", "Keys on this computer")}>
					<KeysCell row={row} />
				</Td>
				<Td label={t("keys.col.backup", "Account backup")}>
					<BackupCell row={row} flows={flows} />
				</Td>
				<Td label={t("keys.col.file", "Backup file")}>
					<FileCell row={row} files={files} />
				</Td>
				<Td label={t("keys.col.identity", "Identity trusted since")}>
					<IdentityCell row={row} />
				</Td>
				<Td label={t("keys.col.actions", "Actions")} kind="more">
					<RowMenu row={row} flows={flows} />
				</Td>
			</Tr>
			<ResultRow scope={rowScope(row.deviceId)} results={results} colSpan={7} />
		</>
	);
}

/** Why the table's key tools are off: the device list is not there yet (R6), or no keys are stored here. */
function toolsGate(t: DevicesT, listLoaded: boolean, anyKeys: boolean) {
	if (!listLoaded)
		return {
			kind: "busy" as const,
			reason: t(
				"devices:keys.computer.listPending",
				"Available once your device list has loaded.",
			),
		};
	return anyKeys
		? null
		: {
				kind: "nokeys" as const,
				reason: t(
					"devices:keys.table.noKeysYet",
					"No keys on this computer yet.",
				),
			};
}

/** SPEC §5.9 "Keys per device": riskiest first, capped, with the row actions and results. */
export function KeysTable({
	read,
	files,
	filter,
	onFilter,
	focusDeviceId,
	flows,
	groupLabel,
}: Readonly<{
	read: KeysRead;
	files: KeyFileLog;
	filter: KeyGroup | null;
	onFilter(group: KeyGroup | null): void;
	focusDeviceId?: string;
	flows: KeyFlowsApi;
	groupLabel(group: KeyGroup): string;
}>) {
	const { t } = useTranslation("devices");
	const results = useKeyResults();
	const [cap, setCap] = useState(FIRST_PAGE);
	const { model, backups, devices } = read;
	const all = model.rows;
	const list = filter
		? all.filter((row) => keyGroupOf(row.category) === filter)
		: all;
	const focusIndex = focusDeviceId
		? list.findIndex((row) => row.deviceId === focusDeviceId)
		: -1;
	const shown = list.slice(0, Math.max(cap, focusIndex + 1));
	const anyKeys = rowsWithKeys(model).length > 0;
	const noKeysGate = toolsGate(t, devices.loaded, anyKeys);
	const head = (
		<tr>
			<Th>{t("keys.col.device", "Device")}</Th>
			<Th>{t("keys.col.relationship", "Relationship")}</Th>
			<Th>{t("keys.col.keys", "Keys on this computer")}</Th>
			<Th>{t("keys.col.backup", "Account backup")}</Th>
			<Th>{t("keys.col.file", "Backup file")}</Th>
			<Th>{t("keys.col.identity", "Identity trusted since")}</Th>
			<Th>
				<span className="sr-only">{t("keys.col.actions", "Actions")}</span>
			</Th>
		</tr>
	);
	return (
		<Block
			id="keys-table"
			icon={KeyRound}
			title={t("keys.table.title", "Keys per device")}
			count={devices.loaded ? all.length : undefined}
			flush
			className="scroll-mt-16"
			stamp={
				<>
					<FreshnessStamp
						source="local"
						age="current"
						text={t("keys.stamp.local", "keys stored in this app")}
					/>
					<BackupsStamp read={read} />
				</>
			}
			tools={
				<>
					<GatedAction gate={noKeysGate}>
						<DvButton
							size="sm"
							icon={Download}
							onClick={() => flows.open({ kind: "download" })}
						>
							{t("keys.table.download", "Download backup files…")}
						</DvButton>
					</GatedAction>
					<GatedAction gate={noKeysGate}>
						<DvButton
							size="sm"
							icon={KeyRound}
							onClick={() => flows.open({ kind: "password" })}
						>
							{t("keys.table.password", "Change device password…")}
						</DvButton>
					</GatedAction>
				</>
			}
			toolbar={
				filter ? (
					<>
						<span className="text-xs text-muted-foreground">
							{t("keys.table.filtered", "Showing {{shown}} of {{total}}", {
								shown: list.length,
								total: all.length,
							})}
						</span>
						<FilterChip pressed onPressedChange={() => onFilter(null)}>
							{groupLabel(filter)}
							<X aria-hidden className="size-3" />
						</FilterChip>
					</>
				) : undefined
			}
			foot={t(
				"keys.table.foot",
				"Keys, backup files and trust dates are stored in this app on this computer. Account backup versions come from your account on the hub. Identity trusted since is when this computer first checked the device's keys against the hub.",
			)}
		>
			{!devices.loaded ? (
				<div className="p-4">
					<StateView
						kind={devices.freshness.age === "error" ? "error" : "loading"}
						title={
							devices.freshness.age === "error"
								? t(
										"keys.table.listError",
										"Couldn't load your device list from the hub.",
									)
								: t("keys.table.loading", "Loading your devices…")
						}
						text={
							devices.freshness.age === "error"
								? t(
										"keys.table.listErrorText",
										"The keys on this computer are safe. The list returns once the hub answers.",
									)
								: undefined
						}
					/>
				</div>
			) : shown.length === 0 ? (
				<div className="p-4">
					{filter ? (
						<StateView
							kind="empty"
							icon={Funnel}
							title={t("keys.table.emptyGroup", "No device in this group")}
							text={t("keys.table.emptyGroupText", {
								count: all.length,
								defaultValue_one: "{{count, number}} device in total.",
								defaultValue_other: "{{count, number}} devices in total.",
							})}
							actions={
								<DvButton size="sm" onClick={() => onFilter(null)}>
									{t("keys.table.showAll", "Show all devices")}
								</DvButton>
							}
						/>
					) : (
						<StateView
							kind="empty"
							title={t("keys.table.empty", "No devices in your list yet")}
							text={t(
								"keys.table.emptyText",
								"Keys appear here once you set up a device or someone shares one with you.",
							)}
						/>
					)}
				</div>
			) : (
				<>
					<DvTable
						cols={KEYS_COLS}
						className={KEYS_TABLE_CLASS}
						head={head}
						label={t(
							"keys.table.caption",
							"Keys on this computer per device, riskiest first",
						)}
					>
						{shown.map((row) => (
							<DeviceRowView
								key={row.deviceId}
								row={row}
								files={files}
								flows={flows}
								results={results}
								focused={row.deviceId === focusDeviceId}
							/>
						))}
					</DvTable>
					{list.length > shown.length ? (
						<div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
							<span>
								{t(
									"keys.table.showing",
									"Showing {{shown}} of {{total}} · riskiest first",
									{ shown: shown.length, total: list.length },
								)}
							</span>
							<DvButton
								size="sm"
								onClick={() => setCap(shown.length + NEXT_PAGE)}
							>
								{t("keys.table.more", "Show {{count, number}} more", {
									count: Math.min(NEXT_PAGE, list.length - shown.length),
								})}
							</DvButton>
						</div>
					) : null}
				</>
			)}
			{backups.interim && model.unchecked.length > 0 && !backups.loading ? (
				<p className="border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					{t("keys.table.interimCap", {
						count: model.unchecked.length,
						defaultValue_one:
							"{{count, number}} device isn't compared with your account yet.",
						defaultValue_other:
							"{{count, number}} devices aren't compared with your account yet.",
					})}
				</p>
			) : null}
		</Block>
	);
}

/** "account backups checked 8 min ago"; a failed read keeps the versions and says so (R5). */
export function BackupsStamp({ read }: Readonly<{ read: KeysRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { backups } = read;
	if (backups.failed)
		return (
			<FreshnessStamp
				source="hub"
				age="error"
				error={
					backups.checkedAt === undefined ? {} : { dataFrom: backups.checkedAt }
				}
			/>
		);
	if (backups.checkedAt === undefined)
		return (
			<FreshnessStamp
				source="hub"
				age="notloaded"
				text={t("keys.stamp.backupsLoading", "checking account backups…")}
			/>
		);
	return (
		<FreshnessStamp
			source="hub"
			age="current"
			observedAt={backups.checkedAt}
			text={t("keys.stamp.backups", "account backups checked {{ago}}", {
				ago: time.ago(Math.min(backups.checkedAt, time.nowS)),
			})}
		/>
	);
}

function localOnlyWhy(
	t: DevicesT,
	row: LocalOnlyRow,
): { text: string; sub?: string } {
	if (row.reason === "revoked")
		return {
			text: t(
				"keys.localOnly.revoked",
				"Revoked. The hub refuses this device, so these keys can't be used any more.",
			),
		};
	if (row.reason === "request")
		return {
			text: t(
				"keys.localOnly.request",
				"Your access request isn't approved yet. These keys exist only here, and can't be backed up to your account until the owner approves.",
			),
			sub: t(
				"keys.localOnly.requestSub",
				"You'll know it's approved when {{device}} appears in your device list.",
				{ device: row.name },
			),
		};
	return {
		text: t(
			"keys.localOnly.unlisted",
			"This device isn't in your list on this hub. It may have been removed, or your access ended.",
		),
	};
}

/** SPEC §5.9 "Local-only keys": keys for revoked devices, unapproved requests and devices not in the list. */
export function LocalOnlyKeys({
	read,
	flows,
}: Readonly<{ read: KeysRead; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const results = useKeyResults();
	const link = useRouteLink();
	const [showAll, setShowAll] = useState(false);
	const rows = read.model.localOnly;
	const shown = showAll ? rows : rows.slice(0, LOCAL_ONLY_CAP);
	const kept = t("keys.localOnly.col.since", "Kept since");
	const head = (
		<tr>
			<Th>{t("keys.localOnly.col.for", "Keys for")}</Th>
			<Th>{t("keys.localOnly.col.why", "Why they're here")}</Th>
			<Th>{kept}</Th>
			<Th>{t("keys.localOnly.col.action", "Action")}</Th>
		</tr>
	);
	return (
		<Block
			id="local-only"
			icon={FolderKey}
			title={t("keys.localOnly.title", "Local-only keys")}
			count={read.devices.loaded ? rows.length : undefined}
			flush
			className="scroll-mt-16"
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("keys.stamp.localShort", "stored in this app")}
				/>
			}
			foot={t(
				"keys.localOnly.foot",
				"Keys for other accounts, hubs or app profiles are kept separately and don't show here.",
			)}
		>
			{!read.devices.loaded ? (
				<div className="p-4">
					<StateView
						kind="notloaded"
						title={t(
							"keys.localOnly.notLoaded",
							"Waiting for your device list",
						)}
						text={t(
							"keys.localOnly.notLoadedText",
							"Leftover keys can only be told apart once the hub has answered.",
						)}
					/>
				</div>
			) : rows.length === 0 ? (
				<div className="p-4">
					<StateView
						kind="empty"
						icon={CircleCheck}
						title={t(
							"keys.localOnly.empty",
							"No leftover keys on this computer",
						)}
						text={t(
							"keys.localOnly.emptyText",
							"Keys for revoked devices and for access requests that aren't approved yet would show here.",
						)}
					/>
				</div>
			) : (
				<DvTable
					cols={LOCAL_COLS}
					className={KEYS_TABLE_CLASS}
					head={head}
					label={t(
						"keys.localOnly.caption",
						"Keys stored here that can't be used for a device in your list",
					)}
				>
					{shown.map((row) => {
						const why = localOnlyWhy(t, row);
						const since = row.since ?? row.registeredAt;
						return (
							<Tr key={row.deviceId} data-local-only={row.reason}>
								<Td label={t("keys.localOnly.col.for", "Keys for")} kind="name">
									<span
										title={row.name}
										className="block truncate font-mono text-[12.5px] font-semibold"
									>
										{row.name}
									</span>
									<CellSub>
										{row.reason === "request"
											? t("keys.localOnly.kindRequest", "{{kind}} · request", {
													kind: keyKindLabel(t, row.role),
												})
											: keyKindLabel(t, row.role)}{" "}
										·{" "}
										<IdRef
											id={row.deviceId}
											copyLabel={t("keys.col.copyId", "Copy device ID")}
										/>
									</CellSub>
								</Td>
								<Td label={t("keys.localOnly.col.why", "Why they're here")}>
									{why.text}
									{why.sub ? <CellSub>{why.sub}</CellSub> : null}
									{row.lastSeenAt ? (
										<CellSub>
											{t("keys.localOnly.lastCheckIn", "last check-in")}{" "}
											<DayOf atS={row.lastSeenAt} />
										</CellSub>
									) : null}
								</Td>
								<Td label={kept}>
									{since === undefined ? (
										<span className="text-muted-foreground">–</span>
									) : row.reason === "request" ? (
										<time title={time.abs(since)}>{time.at(since)}</time>
									) : (
										<DayOf atS={since} />
									)}
									<CellSub>
										{row.reason === "revoked"
											? row.since
												? t("keys.localOnly.revokedAt", "revoked")
												: t("keys.localOnly.atSetup", "at setup")
											: row.reason === "request"
												? t("keys.localOnly.requestAt", "request created")
												: null}
									</CellSub>
								</Td>
								<Td label={t("keys.localOnly.col.action", "Action")} kind="act">
									<div className="flex flex-wrap items-center gap-2">
										{row.reason === "request" ? (
											<DvButton size="sm" icon={Download} asChild>
												<a {...link({ screen: "access", tab: "shared" })}>
													{t(
														"keys.localOnly.requestAgain",
														"Download request again",
													)}
												</a>
											</DvButton>
										) : null}
										<DvButton
											size="sm"
											variant="danger-ghost"
											icon={Trash2}
											busy={flows.busy(row.deviceId)}
											onClick={() => flows.deleteKeys(row)}
										>
											{t("keys.localOnly.delete", "Delete keys…")}
										</DvButton>
									</div>
								</Td>
							</Tr>
						);
					})}
				</DvTable>
			)}
			{rows.length > shown.length ? (
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					<span>
						{t("keys.localOnly.showing", "Showing {{shown}} of {{total}}", {
							shown: shown.length,
							total: rows.length,
						})}
					</span>
					<DvButton size="sm" onClick={() => setShowAll(true)}>
						{t("keys.localOnly.more", "Show {{count, number}} more", {
							count: rows.length - shown.length,
						})}
					</DvButton>
				</div>
			) : null}
			<LocalOnlyResult results={results} />
		</Block>
	);
}

export const LOCAL_ONLY_RESULT = "local-only";

function LocalOnlyResult({ results }: Readonly<{ results: KeyResults }>) {
	const result = results.of(LOCAL_ONLY_RESULT);
	if (!result) return null;
	return (
		<div className="border-t border-hairline px-4 py-3">
			<InlineResult
				tone={result.tone}
				onDismiss={() => results.dismiss(LOCAL_ONLY_RESULT)}
			>
				{result.text}
			</InlineResult>
		</div>
	);
}
