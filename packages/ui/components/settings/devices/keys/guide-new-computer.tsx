"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	ChevronUp,
	CircleCheck,
	FileUp,
	LifeBuoy,
	LockOpen,
	RotateCcw,
} from "lucide-react";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { StatusChip } from "../primitives/status-chip";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import type { KeyFlowsApi } from "./key-flows";
import { type GuideStep, GuideSteps, useNames } from "./key-parts";
import { BackupsStamp } from "./keys-table";
import type { KeysRead } from "./use-keys-model";

function hostOf(origin: string): string {
	try {
		return new URL(origin).host;
	} catch {
		return origin;
	}
}

/**
 * SPEC §5.9 guide "New computer or cleared browser": sign in, restore or
 * import, unlock. `prominent` is the `new-computer` state, where it leads the
 * page and carries the page's one primary action.
 */
export function GuideNewComputer({
	read,
	prominent,
	open,
	onToggle,
	flows,
}: Readonly<{
	read: KeysRead;
	prominent: boolean;
	open: boolean;
	onToggle(): void;
	flows: KeyFlowsApi;
}>) {
	const { t } = useTranslation("devices");
	const names = useNames();
	const workspace = useDeviceWorkspace();
	const { scope, profile } = workspace.deps;
	const { model, local } = read;
	const shown = open || prominent;
	const manageable = model.rows.filter(
		(row) => row.relationship !== "cloud_approval",
	);
	const restorable = manageable.filter((row) => (row.hubRevision ?? 0) > 0);
	const noBackup = manageable.filter((row) => row.hubRevision === 0);
	const locked = model.rows.filter(
		(row) => row.vault && row.session?.state === "locked",
	).length;
	const anyKeys = model.rows.some((row) => row.vault);
	const unlockGate = locked
		? null
		: {
				kind: "nokeys" as const,
				reason: anyKeys
					? t(
							"keys.guide.new.allUnlocked",
							"Every device with keys here is unlocked.",
						)
					: t("keys.guide.new.restoreFirst", "Restore or import keys first."),
			};

	const steps: GuideStep[] = [
		{
			id: "sign-in",
			title: (
				<b className="font-semibold">
					{t(
						"keys.guide.new.step1",
						"Sign in to the same account, hub and app profile.",
					)}
				</b>
			),
			hint: t(
				"keys.guide.new.step1Hint",
				"Keys are kept per account, hub and app profile. This computer is signed in on {{hub}}, profile {{profile}}.",
				{
					hub: hostOf(scope.apiOrigin),
					profile: profile.name || scope.profileId,
				},
			),
			actions: (
				<StatusChip tone="good" icon={CircleCheck}>
					{t("keys.guide.new.signedIn", "Signed in here")}
				</StatusChip>
			),
		},
		{
			id: "restore",
			title: (
				<b className="font-semibold">
					{t(
						"keys.guide.new.step2",
						"Restore from your account backup, or import backup files.",
					)}
				</b>
			),
			hint: (
				<>
					{restorable.length
						? t("keys.guide.new.step2Backups", {
								count: restorable.length,
								devices: names(
									restorable.map((row) =>
										t(
											"keys.guide.new.deviceVersion",
											"{{device}} (v{{version}})",
											{
												device: row.name,
												version: row.hubRevision ?? 0,
											},
										),
									),
								),
								defaultValue_one:
									"Your account holds a backup for {{count, number}} device: {{devices}}. You need the device's password.",
								defaultValue_other:
									"Your account holds backups for {{count, number}} devices: {{devices}}. You need each device's password.",
							})
						: read.backups.checkedAt === undefined
							? t(
									"keys.guide.new.step2Checking",
									"Checking which backups your account holds…",
								)
							: t(
									"keys.guide.new.step2None",
									"Your account holds no key backups for devices in your list.",
								)}
					{noBackup.length
						? ` ${t("keys.guide.new.step2NoBackup", {
								count: noBackup.length,
								devices: names(noBackup.map((row) => row.name)),
								defaultValue_one:
									"{{devices}} has no account backup: import its backup file, or use the computer that set it up.",
								defaultValue_other:
									"{{devices}} have no account backup: import their backup files, or use the computer that set them up.",
							})}`
						: null}
				</>
			),
			actions: (
				<>
					<DvButton
						variant={prominent ? "primary" : "default"}
						icon={RotateCcw}
						onClick={() => flows.open({ kind: "restore" })}
					>
						{t("keys.header.restore", "Restore keys…")}
					</DvButton>
					<DvButton
						icon={FileUp}
						onClick={() => flows.open({ kind: "import" })}
					>
						{t("keys.header.import", "Import backup files…")}
					</DvButton>
				</>
			),
		},
		{
			id: "unlock",
			title: (
				<b className="font-semibold">{t("keys.guide.new.step3", "Unlock.")}</b>
			),
			hint: t(
				"keys.guide.new.step3Hint",
				"Restored keys start locked. Unlock each device with its password. The first unlock refreshes this computer's identity for shared live metrics, so an owner may need to approve it again.",
			),
			actions: (
				<GatedAction gate={unlockGate}>
					<DvButton icon={LockOpen} onClick={() => flows.unlockSeveral()}>
						{t("keys.unlockSeveral", "Unlock several…")}
					</DvButton>
				</GatedAction>
			),
		},
	];

	return (
		<Block
			id="guide-new-computer"
			icon={LifeBuoy}
			className="scroll-mt-16"
			title={
				prominent
					? t(
							"keys.guide.new.titleProminent",
							"Start here: bring your keys to this computer",
						)
					: t("keys.guide.new.title", "New computer or cleared browser")
			}
			stamp={<BackupsStamp read={read} />}
		>
			<p className="max-w-[72ch] text-ui text-ink-2">
				{t(
					"keys.guide.new.lead",
					"Keys live in this app, on one computer. On a new computer, or after this browser's data was cleared, bring them back in three steps.",
				)}
			</p>
			{shown ? (
				<>
					<GuideSteps id="guide-new-steps" steps={steps} />
					{local.platform === "web" ? (
						<p className="text-xs text-muted-foreground">
							{t(
								"keys.guide.new.web",
								"In a browser, choose Keep keys safely afterwards so the browser doesn't delete them again.",
							)}
						</p>
					) : null}
					{prominent ? null : (
						<div>
							<DvButton
								size="sm"
								icon={ChevronUp}
								aria-expanded
								aria-controls="guide-new-steps"
								onClick={onToggle}
							>
								{t("keys.guide.hide", "Hide steps")}
							</DvButton>
						</div>
					)}
				</>
			) : (
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2">
					<ol
						aria-hidden
						className="m-0 flex list-none flex-wrap items-center gap-x-1.5 gap-y-1 p-0 text-xs text-muted-foreground"
					>
						{[
							t("keys.guide.new.mini1", "Sign in"),
							t("keys.guide.new.mini2", "Restore or import"),
							t("keys.guide.new.mini3", "Unlock"),
						].map((label, index) => (
							<li key={label} className="inline-flex items-center gap-1.5">
								{index > 0 ? (
									<span className="text-border-strong">→</span>
								) : null}
								<span className="font-mono text-[11px] font-medium text-ink-2">
									{index + 1}
								</span>
								{label}
							</li>
						))}
					</ol>
					<DvButton
						size="sm"
						icon={ChevronDown}
						aria-expanded={false}
						onClick={onToggle}
					>
						{t("keys.guide.new.show", "Show the 3 steps")}
					</DvButton>
				</div>
			)}
		</Block>
	);
}
