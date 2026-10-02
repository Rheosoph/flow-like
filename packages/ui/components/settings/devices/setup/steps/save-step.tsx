"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CloudOff,
	CloudUpload,
	Download,
	FileKey,
	Fingerprint,
	KeyRound,
	type LucideIcon,
	Package,
	PackageX,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, type Ref, useCallback } from "react";
import { humanFileSize } from "../../../../../lib/utils";
import { enumLabel } from "../../copy/enum-labels";
import { keyFileLogOf } from "../../keys/key-store";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { CheckField } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GatedAction } from "../../primitives/gate-notice";
import { IdRef } from "../../primitives/id-ref";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { PresenceGlyph } from "../../primitives/presence-glyph";
import { StateView } from "../../primitives/state-view";
import { StatusChip } from "../../primitives/status-chip";
import type { ChipTone } from "../../primitives/tone";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDeviceWorkspace, useLocalSummary } from "../../workspace";
import { useSetup } from "../setup-context";
import { Mono } from "../setup-parts";
import {
	type BackupOutcome,
	type CreatedSetup,
	STEP_COUNT,
	keyBackupFile,
	packageFile,
} from "../setup-state";
import { megabytes } from "./platform-step";

const OUTCOME_LOOK: Record<
	BackupOutcome,
	{ tone: ChipTone; icon: LucideIcon }
> = {
	saved: { tone: "good", icon: CloudUpload },
	local_only: { tone: "warning", icon: CloudOff },
	limit: { tone: "warning", icon: CloudOff },
	off: { tone: "outline", icon: CloudOff },
};

export function outcomeLabel(t: DevicesT, outcome: BackupOutcome): string {
	return outcome === "off"
		? t("devices:setup.save.backup.off", "Account backup off")
		: enumLabel(t, "setupBackup", outcome);
}

function OutcomeText({
	outcome,
	name,
}: Readonly<{ outcome: BackupOutcome; name: string }>) {
	const { t } = useTranslation("devices");
	const components = { 1: <Mono>{name}</Mono> };
	switch (outcome) {
		case "saved":
			return (
				<Trans
					t={t}
					i18nKey="setup.save.backup.savedText"
					defaults="Encrypted keys for <1/> are backed up to your account. You still need the device password to restore them."
					components={components}
				/>
			);
		case "local_only":
			return (
				<Trans
					t={t}
					i18nKey="setup.save.backup.localText"
					defaults="The account backup didn't finish, so <1/>'s keys are only on this computer. Save the key backup file below; you can back them up to your account from Keys & recovery once the device has checked in."
					components={components}
				/>
			);
		case "limit":
			return (
				<Trans
					t={t}
					i18nKey="setup.save.backup.limitText"
					defaults="The hub refused another account backup: your account reached its backup limit or sent too many requests. Save the key backup file below; you can retry later from Keys & recovery."
					components={components}
				/>
			);
		default:
			return (
				<Trans
					t={t}
					i18nKey="setup.save.backup.offText"
					defaults="You turned the account backup off. The key backup file below is the only copy of <1/>'s keys outside this computer."
					components={components}
				/>
			);
	}
}

function BackupBlock({
	created,
	outcome,
}: Readonly<{ created: CreatedSetup; outcome: BackupOutcome }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, leaveLink } = useSetup();
	const look = OUTCOME_LOOK[outcome];
	return (
		<Block
			icon={CloudUpload}
			title={t("setup.field.backup", "Account backup")}
			summary={
				<StatusChip tone={look.tone} icon={look.icon}>
					{outcomeLabel(t, outcome)}
				</StatusChip>
			}
			stamp={
				outcome === "off" ? (
					<FreshnessStamp
						source="local"
						age="current"
						text={t("setup.save.backup.choice", "your choice")}
					/>
				) : (
					<FreshnessStamp
						source="hub"
						age="current"
						noFail
						observedAt={created.createdAt}
						text={t("setup.save.backup.answered", "answered {{ago}}", {
							ago: time.ago(created.createdAt),
						})}
					/>
				)
			}
			bodyClassName="items-start"
		>
			<p className="text-ui">
				<OutcomeText outcome={outcome} name={draft.name} />
			</p>
			{outcome === "local_only" || outcome === "limit" ? (
				<DvButton size="sm" icon={KeyRound} asChild>
					<a {...leaveLink({ screen: "keys" })}>
						{t("setup.save.backup.openKeys", "Open Keys & recovery")}
					</a>
				</DvButton>
			) : null}
		</Block>
	);
}

function DownloadRow({
	icon: Icon,
	title,
	chip,
	file,
	hint,
	action,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	chip?: ReactNode;
	file: string;
	hint: ReactNode;
	action: ReactNode;
}>) {
	return (
		<li className="grid grid-cols-[20px_minmax(0,1fr)] items-start gap-x-3 gap-y-2 border-t border-hairline py-3 first:border-t-0 first:pt-0.5 last:pb-0.5 @[560px]/setup:grid-cols-[20px_minmax(0,1fr)_auto]">
			<Icon aria-hidden className="mt-0.5 size-4 text-muted-foreground" />
			<div className="flex min-w-0 flex-col gap-0.75">
				<span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui font-semibold">
					{title}
					{chip}
				</span>
				<span className="truncate font-mono text-xs text-ink-2" title={file}>
					{file}
				</span>
				<p className="text-xs text-muted-foreground">{hint}</p>
			</div>
			<div className="col-start-2 flex @[560px]/setup:col-start-3 @[560px]/setup:justify-end">
				{action}
			</div>
		</li>
	);
}

function KeyChip({
	required,
	saved,
	risky,
}: Readonly<{ required: boolean; saved: boolean; risky: boolean }>) {
	const { t } = useTranslation("devices");
	if (saved)
		return (
			<StatusChip tone="good" icon={CircleCheck}>
				{t("setup.save.chip.saved", "Saved")}
			</StatusChip>
		);
	if (required)
		return (
			<StatusChip tone="warning" icon={TriangleAlert}>
				{t("setup.save.chip.required", "Required")}
			</StatusChip>
		);
	return risky ? (
		<StatusChip tone="warning" icon={TriangleAlert}>
			{t("setup.save.chip.recommended", "Recommended")}
		</StatusChip>
	) : (
		<StatusChip tone="outline">
			{t("setup.save.chip.optional", "Optional · your account has a copy")}
		</StatusChip>
	);
}

/** Handing out the key backup file: the Save step notes it, and so does this computer's record for Keys & recovery. */
function useKeySaved(deviceId: string) {
	const { update } = useSetup();
	const workspace = useDeviceWorkspace();
	return useCallback(() => {
		update({ keySaved: true });
		keyFileLogOf(workspace).update(deviceId, {
			savedAt: workspace.clock.now(),
		});
	}, [update, workspace, deviceId]);
}

/** Both files, only while this window still holds them. */
function Downloads({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const { draft, update, built, option } = useSetup();
	const local = useLocalSummary();
	const keySaved = useKeySaved(created.deviceId);
	if (!built) return null;
	const needKey = created.outcome !== "saved";
	const risky =
		local.platform !== "desktop" && local.persistence !== "persisted";
	const blocked = needKey && !draft.keySaved;
	const packageName = packageFile(draft.name);
	const keyName = keyBackupFile(draft.name);
	const packageLabel = t(
		"setup.save.package.download",
		"Download setup package",
	);
	return (
		<ul className="m-0 flex list-none flex-col p-0">
			<DownloadRow
				icon={FileKey}
				title={t("setup.save.key.title", "Key backup file")}
				chip={
					<KeyChip required={needKey} saved={draft.keySaved} risky={risky} />
				}
				file={t("setup.save.file", "{{file}} · {{size}}", {
					file: keyName,
					size: humanFileSize(built.backupBytes),
				})}
				hint={t(
					"setup.save.key.hint",
					"Opens only with the device password. Keep it somewhere other than this computer.",
				)}
				action={
					<DvButton icon={Download} asChild>
						<a href={built.backupUrl} download={keyName} onClick={keySaved}>
							{t("setup.save.key.download", "Download key backup file")}
						</a>
					</DvButton>
				}
			/>
			<DownloadRow
				icon={Package}
				title={t("setup.save.package.title", "Setup package")}
				chip={
					draft.packageSaved ? (
						<StatusChip tone="good" icon={CircleCheck}>
							{t("setup.save.chip.saved", "Saved")}
						</StatusChip>
					) : null
				}
				file={t("setup.save.file", "{{file}} · {{size}}", {
					file: packageName,
					size:
						option?.size && built.packageBytes > 1_000_000
							? megabytes(built.packageBytes)
							: humanFileSize(built.packageBytes),
				})}
				hint={
					<Trans
						t={t}
						i18nKey="setup.save.package.hint"
						defaults="The agent, start scripts, the signed release and a one-time setup secret for <1/>. Anyone with it can register this device until it's used, so treat it like a key."
						components={{ 1: <Mono>{draft.name}</Mono> }}
					/>
				}
				action={
					blocked ? (
						<GatedAction
							gate={{
								kind: "nokeys",
								reason: t(
									"setup.save.package.gate",
									"Save the key backup file first. Without it or an account backup, losing this computer's storage means setting the device up again.",
								),
							}}
							className="@[560px]/setup:items-end"
						>
							<DvButton icon={Download}>{packageLabel}</DvButton>
						</GatedAction>
					) : (
						<DvButton icon={Download} asChild>
							<a
								href={built.packageUrl}
								download={packageName}
								onClick={() => update({ packageSaved: true })}
							>
								{packageLabel}
							</a>
						</DvButton>
					)
				}
			/>
		</ul>
	);
}

/** A window that was reloaded, or that resumed a setup made elsewhere: the package was handed out once. */
function NotInThisWindow({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, update } = useSetup();
	return (
		<>
			<StateView
				kind="notloaded"
				icon={PackageX}
				title={t(
					"setup.save.gone.title",
					"This window can't download the package again",
				)}
				text={t(
					"setup.save.gone.text",
					"It was built {{when}} and is only ever handed out once. If you saved it then, carry on. If not, cancel this setup and create a new one.",
					{ when: time.ago(created.createdAt) },
				)}
			/>
			<CheckField
				id="dv-setup-have-package"
				checked={draft.acknowledged}
				onCheckedChange={(acknowledged) => update({ acknowledged })}
				className="rounded-lg border border-border bg-card px-3 py-2.5"
			>
				<Trans
					t={t}
					i18nKey="setup.save.gone.ack"
					defaults="I have <1/> from when it was created"
					components={{ 1: <Mono>{packageFile(draft.name)}</Mono> }}
				/>
			</CheckField>
		</>
	);
}

function DeviceFacts({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { limits } = useSetup();
	return (
		<Block
			icon={Fingerprint}
			title={t("setup.save.device.title", "The new device")}
			stamp={
				<FreshnessStamp
					source="hub"
					age="current"
					noFail
					observedAt={created.createdAt}
					text={t("setup.save.device.registered", "registered {{ago}}", {
						ago: time.ago(created.createdAt),
					})}
				/>
			}
		>
			<KeyValueList>
				<KvRow
					label={t("setup.field.deviceId", "Device ID")}
					provenance={t("setup.save.device.permanent", "permanent")}
				>
					<IdRef
						id={created.deviceId}
						copyLabel={t("setup.copy.deviceId", "Copy device ID")}
					/>
				</KvRow>
				{created.enrollmentId ? (
					<KvRow label={t("setup.field.setupId", "Setup ID")}>
						<IdRef
							id={created.enrollmentId}
							copyLabel={t("setup.copy.setupId", "Copy setup ID")}
						/>
					</KvRow>
				) : null}
				<KvRow
					label={t("setup.save.device.before", "Start it before")}
					provenance={t(
						"setup.save.device.then",
						"then the package stops working",
					)}
				>
					<b className="font-semibold">{time.at(created.expiresAt)}</b> (
					{time.ago(created.expiresAt)})
				</KvRow>
				<KvRow label={t("setup.save.device.status", "Status")}>
					<span className="inline-flex items-center gap-1.5">
						<PresenceGlyph kind="pending" decorative />
						{limits.maxPending === undefined
							? t(
									"setup.save.device.waitingPlain",
									"Waiting for the device · counts toward your unused packages",
								)
							: t(
									"setup.save.device.waiting",
									"Waiting for the device · counts toward your {{max, number}} unused packages",
									{ max: limits.maxPending },
								)}
					</span>
				</KvRow>
			</KeyValueList>
		</Block>
	);
}

/** Step 5: the two files that exist only now, and the facts of the registered device. */
export function SaveStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { draft, built } = useSetup();
	const local = useLocalSummary();
	const created = draft.created;
	if (!created) return null;
	const risky =
		local.platform !== "desktop" && local.persistence !== "persisted";
	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={6}
				total={STEP_COUNT}
				title={t("setup.save.title", "Save the package and key backup")}
			/>
			{risky && built ? (
				<Banner
					tone="warning"
					title={t(
						"setup.save.risk.title",
						"This browser may delete these keys.",
					)}
				>
					{t(
						"setup.save.risk.text",
						"Save the key backup file even if your account has a copy: it's the quickest way back if the browser clears the keys.",
					)}
				</Banner>
			) : null}
			{created.outcome ? (
				<BackupBlock created={created} outcome={created.outcome} />
			) : null}
			<Block
				icon={Download}
				title={t("setup.save.files.title", "Save to this computer")}
				stamp={
					<FreshnessStamp
						source="local"
						age={built ? "current" : "notloaded"}
						text={
							built
								? t("setup.save.files.here", "built in this window")
								: t("setup.save.files.gone", "not in this window")
						}
					/>
				}
				foot={t(
					"setup.save.files.foot",
					"This package can't be downloaded again. If you lose it, cancel this setup and create a new one.",
				)}
			>
				{built ? (
					<Downloads created={created} />
				) : (
					<NotInThisWindow created={created} />
				)}
			</Block>
			<DeviceFacts created={created} />
		</>
	);
}
