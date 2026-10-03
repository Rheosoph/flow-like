"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ChevronRight,
	CircleCheck,
	CloudOff,
	CloudUpload,
	Download,
	FileUp,
	KeyRound,
	LoaderCircle,
	type LucideIcon,
	RefreshCw,
	RotateCcw,
	ShieldCheck,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import type {
	ActionId,
	DeviceRow,
} from "../../../../lib/device-management/model/types";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../../ui/collapsible";
import type { BackupMode } from "../keys/account-backup-panel";
import { type KeyFlowsApi, useKeyFlows } from "../keys/key-flows";
import {
	KvHint,
	Mono,
	accountVersionLabel,
	dayLabel,
	keyChipOf,
	keyKindLabel,
} from "../keys/key-parts";
import {
	type KeyFileLog,
	rowScope,
	useKeyFileLog,
	useKeyResults,
} from "../keys/key-store";
import {
	type KeyRow,
	type LocalOnlyRow,
	localRevision,
} from "../keys/keys-model";
import {
	BackupsStamp,
	Fingerprint,
	LOCAL_ONLY_RESULT,
} from "../keys/keys-table";
import { storageAtRisk, storageLabel } from "../keys/persistence-notice";
import { useTrustChain } from "../keys/trust-chain-links";
import { useKeyGate } from "../keys/use-key-actions";
import { type KeysRead, useKeysModel } from "../keys/use-keys-model";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { RequestedActual } from "../primitives/requested-actual";
import { StateView } from "../primitives/state-view";
import { KeyChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { TrustChain } from "../primitives/trust-chain";
import { useRouteLink } from "../routing/use-devices-route";
import type { DeviceTabProps } from "../screen-props";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useDeviceRow } from "../workspace/use-hub";

function hostOf(origin: string): string {
	try {
		return new URL(origin).host;
	} catch {
		return origin;
	}
}

function useDateOf(): (atS: number) => string {
	const time = useAreaTime();
	return (atS) => dayLabel(atS, time.now, time.locale);
}

function ChainBlock({
	device,
	row,
}: Readonly<{ device: DeviceRow; row: KeyRow | undefined }>) {
	const { t } = useTranslation("devices");
	const links = useTrustChain(device, row);
	return (
		<Block
			icon={ShieldCheck}
			title={t("keys.tab.chain.title", "Trust chain")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("keys.tab.chain.stamp", "checked on this computer")}
				/>
			}
		>
			<TrustChain links={links} />
		</Block>
	);
}

function BackupFact({ row, read }: Readonly<{ row: KeyRow; read: KeysRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hub = row.hubRevision ?? 0;
	const local = localRevision(row);
	const here = t("keys.backup.here", "This computer v{{version}}", {
		version: local || 1,
	});
	const looks: Record<
		KeyRow["category"],
		{
			tone?: "good" | "info" | "warning";
			icon?: LucideIcon;
			sub: string;
		} | null
	> = {
		synced: {
			tone: "good",
			icon: CircleCheck,
			sub: t("keys.tab.backup.synced", "In sync"),
		},
		pending: {
			tone: "info",
			icon: LoaderCircle,
			sub: t("keys.state.pending", "Upload pending"),
		},
		oldpw: {
			tone: "warning",
			icon: TriangleAlert,
			sub: t(
				"keys.backup.oldpwSub",
				"The account copy still opens with the old password.",
			),
		},
		out_of_date: {
			tone: "warning",
			icon: CloudOff,
			sub: t(
				"keys.backup.outOfDateSub",
				"The keys here changed after v{{version}} was saved.",
				{ version: hub },
			),
		},
		hub_newer: {
			tone: "warning",
			icon: TriangleAlert,
			sub: t(
				"keys.backup.hubNewerSub",
				"Another computer saved a newer backup. Check it to adopt its version.",
			),
		},
		never: {
			tone: "warning",
			icon: CloudOff,
			sub: t(
				"keys.tab.backup.never",
				"Not backed up. The keys exist only here.",
			),
		},
		unchecked: null,
		restorable: null,
		lost: null,
		nokeys: null,
	};
	const look = looks[row.category];
	return (
		<KvRow label={t("keys.col.backup", "Account backup")}>
			{look ? (
				<RequestedActual
					versions
					requested={here}
					actual={accountVersionLabel(t, row.hubRevision)}
					label={
						hub
							? t(
									"keys.backup.pendingAria",
									"This computer has version {{local}}; your account has version {{hub}}",
									{ local: local || 1, hub },
								)
							: t(
									"keys.backup.pendingFirstAria",
									"This computer has version {{local}}; it hasn't reached your account yet",
									{ local: local || 1 },
								)
					}
					tone={look.tone}
					icon={look.icon}
					sub={look.sub}
				/>
			) : (
				<span className="text-muted-foreground">
					{t("keys.state.unchecked", "Not checked yet")}
				</span>
			)}
			{row.hubSavedAt ? (
				<KvHint>
					<span title={time.abs(row.hubSavedAt)}>
						{t("keys.tab.backup.saved", "Last saved {{ago}}.", {
							ago: time.ago(row.hubSavedAt, "long"),
						})}
					</span>{" "}
					{read.backups.slots
						? t(
								"keys.tab.backup.slots",
								"Your account uses {{used, number}} of {{max, number}} backup slots.",
								read.backups.slots,
							)
						: null}
				</KvHint>
			) : read.backups.interim ? (
				<KvHint>
					{t(
						"keys.tab.backup.interim",
						"This hub reports the version only, not when it was saved.",
					)}
				</KvHint>
			) : null}
		</KvRow>
	);
}

function GatedButton({
	action,
	deviceId,
	icon,
	busy,
	danger,
	size = "md",
	onClick,
	children,
}: Readonly<{
	action: ActionId;
	deviceId: string;
	icon?: LucideIcon;
	busy?: boolean;
	danger?: boolean;
	size?: "md" | "sm";
	onClick(): void;
	children: ReactNode;
}>) {
	const gate = useKeyGate(action, deviceId);
	return (
		<GatedAction gate={gate}>
			<DvButton
				size={size}
				icon={icon}
				busy={busy}
				variant={danger ? "danger-ghost" : "default"}
				onClick={onClick}
			>
				{children}
			</DvButton>
		</GatedAction>
	);
}

function RowResult({ scope }: Readonly<{ scope: string }>) {
	const results = useKeyResults();
	const result = results.of(scope);
	if (!result) return null;
	return (
		<InlineResult tone={result.tone} onDismiss={() => results.dismiss(scope)}>
			{result.text}
		</InlineResult>
	);
}

function KeysBlock({
	row,
	read,
	files,
	flows,
}: Readonly<{
	row: KeyRow;
	read: KeysRead;
	files: KeyFileLog;
	flows: KeyFlowsApi;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const dateOf = useDateOf();
	const link = useRouteLink();
	const workspace = useDeviceWorkspace();
	const { scope, profile } = workspace.deps;
	const { deviceId } = row;
	const vault = row.vault;
	if (!vault) return null;
	const record = files[deviceId];
	const risk = storageAtRisk(read.local);
	const backup: { label: string; icon: LucideIcon; run(): void } =
		row.category === "pending"
			? {
					label: t("keys.backup.retry", "Retry upload"),
					icon: RefreshCw,
					run: () => flows.retryUpload(row),
				}
			: row.category === "never"
				? {
						label: t("keys.backup.backUp", "Back up to account"),
						icon: CloudUpload,
						run: () => flows.open({ kind: "backup", deviceId, mode: "save" }),
					}
				: {
						label: t("keys.backup.update", "Update account backup"),
						icon: CloudUpload,
						run: () => flows.open({ kind: "backup", deviceId, mode: "update" }),
					};
	const check: BackupMode = "check";
	return (
		<Block
			icon={KeyRound}
			title={t("keys.tab.block.title", "Keys on this computer")}
			stamp={
				<>
					<FreshnessStamp source="local" age="current" />
					<BackupsStamp read={read} />
				</>
			}
			foot={t(
				"keys.tab.block.foot",
				"The device password never leaves this computer and the hub never sees it. If you forget it, nobody can recover it.",
			)}
		>
			<KeyValueList>
				<KvRow label={t("keys.tab.fact.keys", "Keys")}>
					<span className="inline-flex flex-wrap items-center gap-2">
						{keyKindLabel(t, vault.role)}
						<KeyChip {...keyChipOf(row)} />
					</span>
					{vault.requiresFreshEndpoint ? (
						<KvHint>
							{t(
								"keys.tab.fact.restored",
								"Restored from a backup. The first unlock refreshes this computer's identity for shared live metrics, so an owner may need to approve it again.",
							)}
						</KvHint>
					) : null}
				</KvRow>
				<KvRow label={t("keys.computer.storedFor", "Stored for")}>
					<Trans
						t={t}
						i18nKey="keys.computer.storedForValue"
						defaults="Your account · {{hub}} · app profile <1/>"
						values={{ hub: hostOf(scope.apiOrigin) }}
						components={{ 1: <Mono>{profile.name || scope.profileId}</Mono> }}
					/>
				</KvRow>
				<KvRow label={t("keys.col.identity", "Identity trusted since")}>
					<span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
						{vault.identityPinnedAt === undefined ? (
							t("keys.identity.onFirstUnlock", "On first unlock")
						) : (
							<span title={time.abs(Math.floor(vault.identityPinnedAt / 1000))}>
								{dateOf(Math.floor(vault.identityPinnedAt / 1000))}
							</span>
						)}
						{vault.identityFingerprint ? (
							<Fingerprint value={vault.identityFingerprint} />
						) : null}
					</span>
				</KvRow>
				<BackupFact row={row} read={read} />
				<KvRow
					label={t("keys.col.file", "Backup file")}
					provenance={
						record?.savedAt === undefined
							? undefined
							: t("keys.tab.fact.localRecord", "local record")
					}
				>
					{record?.savedAt === undefined ? (
						t("keys.tab.fact.fileNever", "never downloaded here")
					) : (
						<span title={time.abs(Math.floor(record.savedAt / 1000))}>
							{t(
								"keys.tab.fact.fileSaved",
								"last downloaded on this computer {{date}}",
								{ date: dateOf(Math.floor(record.savedAt / 1000)) },
							)}
						</span>
					)}
					{record?.savedAt !== undefined &&
					record.passwordChangedAt !== undefined &&
					record.passwordChangedAt > record.savedAt ? (
						<KvHint>
							{t(
								"keys.tab.fact.fileOld",
								"That file opens with the old password.",
							)}
						</KvHint>
					) : null}
				</KvRow>
				<KvRow label={t("keys.computer.storage", "Storage")}>
					<span
						className={cx(
							"inline-flex items-center gap-1",
							risk && "text-warning",
						)}
					>
						{risk ? (
							<TriangleAlert aria-hidden className="size-3.5 shrink-0" />
						) : null}
						{storageLabel(t, read.local)}
					</span>
				</KvRow>
			</KeyValueList>
			<div className="flex flex-wrap items-start gap-2">
				<GatedButton
					action="account_backup_save"
					deviceId={deviceId}
					icon={backup.icon}
					busy={flows.busy(deviceId)}
					onClick={backup.run}
				>
					{backup.label}
				</GatedButton>
				<GatedButton
					action="account_backup_save"
					deviceId={deviceId}
					icon={ShieldCheck}
					onClick={() => flows.open({ kind: "backup", deviceId, mode: check })}
				>
					{t("keys.tab.action.check", "Check account backup")}
				</GatedButton>
				<GatedButton
					action="download_key_file"
					deviceId={deviceId}
					icon={Download}
					onClick={() => flows.open({ kind: "download", deviceId })}
				>
					{t("keys.menu.download", "Download key backup file…")}
				</GatedButton>
				<GatedButton
					action="change_device_password"
					deviceId={deviceId}
					icon={KeyRound}
					onClick={() => flows.open({ kind: "password", deviceId })}
				>
					{t("keys.menu.password", "Change device password…")}
				</GatedButton>
			</div>
			<RowResult scope={rowScope(deviceId)} />
			<Collapsible>
				<CollapsibleTrigger className="group inline-flex cursor-pointer items-center gap-1 text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
					<ChevronRight
						aria-hidden
						className="size-3.5 transition-transform group-data-[state=open]:rotate-90"
					/>
					{t("keys.tab.advanced", "Advanced")}
				</CollapsibleTrigger>
				<CollapsibleContent className="pt-2">
					<GatedButton
						action="reset_metric_reader"
						deviceId={deviceId}
						size="sm"
						onClick={() => flows.open({ kind: "reset", deviceId })}
					>
						{t("keys.tab.action.reset", "Reset metric-group identity…")}
					</GatedButton>
				</CollapsibleContent>
			</Collapsible>
			<div className="flex flex-wrap items-center gap-x-3 gap-y-2">
				<GatedButton
					action="delete_local_keys"
					deviceId={deviceId}
					icon={Trash2}
					danger
					busy={flows.busy(deviceId)}
					onClick={() => flows.deleteKeys(row)}
				>
					{t("keys.menu.delete", "Delete keys from this computer…")}
				</GatedButton>
				<a
					{...link({
						screen: "keys",
						guide: "forgot-password",
						focusDeviceId: deviceId,
					})}
					className="text-ui underline decoration-border-strong underline-offset-2 hover:decoration-current"
				>
					{t("keys.tab.forgot", "Forgot the device password?")}
				</a>
			</div>
		</Block>
	);
}

/** SPEC §5.2 Keys tab, no-keys state: the three ways to get keys for this device. */
function NoKeysBlock({
	device,
	row,
	flows,
}: Readonly<{ device: DeviceRow; row: KeyRow; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const deviceId = device.device_id;
	return (
		<Block
			icon={KeyRound}
			title={t("keys.tab.block.title", "Keys on this computer")}
			stamp={<FreshnessStamp source="local" age="current" />}
		>
			<StateView
				kind="gate"
				gate="nokeys"
				title={t(
					"keys.tab.none.title",
					"This computer has no keys for {{device}}.",
					{ device: row.name },
				)}
				text={
					<>
						{t(
							"keys.tab.none.text",
							"Keys are stored per browser or app, per account, per hub and per app profile. They may exist on another computer or profile.",
						)}{" "}
						{row.category === "restorable"
							? t(
									"keys.focus.backup",
									"Your account holds a backup (v{{version}}); restore it with the device password.",
									{ version: row.hubRevision ?? 0 },
								)
							: row.category === "lost"
								? t(
										"keys.focus.noBackup",
										"There's no account backup. Import its backup file, or use the computer that set it up.",
									)
								: null}
					</>
				}
				actions={
					<>
						<GatedButton
							action="account_backup_restore"
							deviceId={deviceId}
							icon={RotateCcw}
							onClick={() => flows.open({ kind: "restore", deviceId })}
						>
							{t("keys.menu.restore", "Restore from account backup…")}
						</GatedButton>
						<GatedButton
							action="import_key_file"
							deviceId={deviceId}
							icon={FileUp}
							onClick={() => flows.open({ kind: "import" })}
						>
							{t("keys.menu.import", "Import backup file…")}
						</GatedButton>
						{row.relationship === "owner" ? null : (
							<DvButton variant="ghost" asChild>
								<a
									{...link({
										screen: "access",
										tab: "shared",
										action: "request",
									})}
								>
									{t("keys.tab.none.request", "Request access")}
								</a>
							</DvButton>
						)}
					</>
				}
			/>
			<RowResult scope={rowScope(deviceId)} />
		</Block>
	);
}

/** A revoked device keeps only the choice to delete what is stored here. */
function RevokedBlock({
	device,
	local,
	flows,
}: Readonly<{
	device: DeviceRow;
	local: LocalOnlyRow | undefined;
	flows: KeyFlowsApi;
}>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			icon={KeyRound}
			title={t("keys.tab.block.title", "Keys on this computer")}
			stamp={<FreshnessStamp source="local" age="current" />}
		>
			{local ? (
				<>
					<KeyValueList>
						<KvRow label={t("keys.tab.fact.keys", "Keys")}>
							{t("keys.tab.revoked.kind", "{{kind}} · for a revoked device", {
								kind: keyKindLabel(t, local.role),
							})}
							<KvHint>
								{t(
									"keys.localOnly.revoked",
									"Revoked. The hub refuses this device, so these keys can't be used any more.",
								)}
							</KvHint>
						</KvRow>
					</KeyValueList>
					<div>
						<DvButton
							variant="danger-ghost"
							icon={Trash2}
							busy={flows.busy(local.deviceId)}
							onClick={() => flows.deleteKeys(local)}
						>
							{t("keys.menu.delete", "Delete keys from this computer…")}
						</DvButton>
					</div>
				</>
			) : (
				<StateView
					kind="empty"
					title={t(
						"keys.tab.revoked.none",
						"This computer holds no keys for {{device}}.",
						{ device: device.display_name || device.name },
					)}
					text={t(
						"keys.tab.revoked.noneText",
						"The device is revoked, so there is nothing to restore or back up.",
					)}
				/>
			)}
			<RowResult scope={LOCAL_ONLY_RESULT} />
		</Block>
	);
}

/** N2 › Keys (SPEC §5.2): the device-scoped view of Keys & recovery. */
export function DeviceKeysTab({ deviceId }: Readonly<DeviceTabProps>) {
	const { t } = useTranslation("devices");
	const device = useDeviceRow(deviceId);
	const read = useKeysModel({ interimFor: [deviceId] });
	const files = useKeyFileLog();
	const { api: flows, sheets } = useKeyFlows(read, files);
	if (!device)
		return (
			<StateView
				kind={read.devices.loaded ? "notloaded" : "loading"}
				title={
					read.devices.loaded
						? t("keys.tab.unknown", "This device isn't in your list")
						: t("keys.tab.loading", "Loading this device's keys…")
				}
			/>
		);
	const revoked = device.status === "revoked";
	const row = read.model.rows.find((entry) => entry.deviceId === deviceId);
	const local = read.model.localOnly.find(
		(entry) => entry.deviceId === deviceId,
	);
	return (
		<div data-tab="keys" className="flex min-w-0 flex-col gap-6">
			{revoked ? (
				<RevokedBlock device={device} local={local} flows={flows} />
			) : row?.vault ? (
				<>
					<ChainBlock device={device} row={row} />
					<KeysBlock row={row} read={read} files={files} flows={flows} />
				</>
			) : row && row.relationship !== "cloud_approval" ? (
				<>
					<NoKeysBlock device={device} row={row} flows={flows} />
					<ChainBlock device={device} row={row} />
				</>
			) : (
				<Block
					icon={KeyRound}
					title={t("keys.tab.block.title", "Keys on this computer")}
					stamp={<FreshnessStamp source="local" age="current" />}
				>
					<StateView
						kind="noaccess"
						title={t("keys.tab.consent.title", "No keys to hold")}
						text={t(
							"keys.tab.consent.text",
							"You only approve or pay for this device's cloud access, so this computer keeps no keys for it.",
						)}
					/>
				</Block>
			)}
			{sheets}
		</div>
	);
}
