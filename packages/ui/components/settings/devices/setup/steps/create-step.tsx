"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	ClipboardList,
	KeyRound,
	LoaderCircle,
	OctagonX,
	Package,
	Server,
} from "lucide-react";
import type { Ref } from "react";
import { enumLabel } from "../../copy/enum-labels";
import { gateCopy } from "../../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { Checklist } from "../../primitives/checklist";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GateNotice } from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StatusChip } from "../../primitives/status-chip";
import { WizardStepHeader } from "../../primitives/wizard";
import { useCreatePhases } from "../create-phases";
import { useSetup } from "../setup-context";
import { GroupLabel, Mono, TechnicalDetails } from "../setup-parts";
import { STEP_COUNT, packageFile, packsAgent } from "../setup-state";
import type { SetupFailure } from "../use-setup-create";
import { megabytes } from "./platform-step";

/** What a failed creation means for the user; the technical sentence stays behind "Details". */
export function failureText(t: DevicesT, failure: SetupFailure): string {
	switch (failure.kind) {
		case "limit":
			return t(
				"devices:setup.create.failure.limit",
				"The hub refused another setup: you reached a limit for devices, unused setup packages or packages per day. Cancel a pending setup, or try again later.",
			);
		case "hub_refused":
			return t(
				"devices:setup.create.failure.hubRefused",
				"The hub refused the request. Nothing was created.",
			);
		case "hub_unreachable":
			return t(
				"devices:setup.create.failure.hubUnreachable",
				"The hub didn't answer. Check your connection and try again.",
			);
		case "download":
			return t(
				"devices:setup.create.failure.download",
				"The agent couldn't be downloaded, or it didn't match the signed release. The setup was cancelled at the hub, so no slot stays reserved.",
			);
		case "cancel_failed":
			return t(
				"devices:setup.create.failure.cancelFailed",
				"Creating failed, and the hub kept the setup reserved. Cancel it in Pending setups, or it expires on its own.",
			);
		case "gated":
			return failure.gate
				? gateCopy(t, failure.gate).title
				: t(
						"devices:setup.create.failure.gated",
						"This hub can't set up a device right now.",
					);
		default:
			return t(
				"devices:setup.create.failure.local",
				"The package couldn't be made on this computer. Nothing stays reserved at the hub.",
			);
	}
}

function Change({ to, label }: Readonly<{ to: 1 | 2 | 3; label: string }>) {
	const { t } = useTranslation("devices");
	const { goTo } = useSetup();
	return (
		<DvButton
			variant="link"
			size="xs"
			aria-label={label}
			className="ml-2 text-muted-foreground"
			onClick={() => goTo(to)}
		>
			{t("setup.create.change", "Change")}
		</DvButton>
	);
}

/** The file to expect and about how large it is: the agent plus scripts, or scripts only. */
function PackageSummary() {
	const { t } = useTranslation("devices");
	const { option, draft } = useSetup();
	const file = { 1: <Mono>{packageFile(draft.name)}</Mono> };
	if (packsAgent(option, draft.mode) && option?.size)
		return (
			<Trans
				t={t}
				i18nKey="setup.create.summary.packageSize"
				defaults="<1/> · about {{size}}"
				values={{ size: megabytes(option.size) }}
				components={file}
			/>
		);
	return option?.deferredDownload && draft.mode !== "docker" ? (
		<Trans
			t={t}
			i18nKey="setup.create.summary.packageDeferred"
			defaults="<1/> · under 1 MB · the agent downloads on first start"
			components={file}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="setup.create.summary.packageSmall"
			defaults="<1/> · under 1 MB"
			components={file}
		/>
	);
}

function Summary() {
	const { t } = useTranslation("devices");
	const { draft, release } = useSetup();
	return (
		<Block
			icon={ClipboardList}
			title={t("setup.create.summary.title", "What will be created")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("setup.create.summary.stamp", "your choices")}
				/>
			}
		>
			<KeyValueList>
				<KvRow label={t("setup.field.name", "Name")}>
					<Mono>{draft.name}</Mono>
					<Change to={1} label={t("setup.create.changeName", "Change name")} />
				</KvRow>
				<KvRow label={t("setup.field.platform", "Platform")}>
					{draft.target ? enumLabel(t, "target", draft.target) : null}
					<Change
						to={2}
						label={t("setup.create.changePlatform", "Change platform")}
					/>
				</KvRow>
				<KvRow label={t("setup.field.mode", "Run with")}>
					{draft.mode ? enumLabel(t, "packageMode", draft.mode) : null}
					<Change
						to={2}
						label={t("setup.create.changeMode", "Change how it runs")}
					/>
				</KvRow>
				{release ? (
					<KvRow label={t("setup.check.release.agent", "Agent")}>
						<Trans
							t={t}
							i18nKey="setup.create.summary.agent"
							defaults="<1>{{version}}</1> · release #{{sequence, number}} · verified"
							values={{
								version: release.manifest.release_version,
								sequence: release.manifest.sequence,
							}}
							components={{ 1: <Mono /> }}
						/>
					</KvRow>
				) : null}
				<KvRow label={t("setup.field.package", "Package")}>
					<PackageSummary />
				</KvRow>
				<KvRow label={t("setup.field.backup", "Account backup")}>
					{draft.backup
						? t(
								"setup.create.summary.backupOn",
								"On · encrypted with the device password",
							)
						: t(
								"setup.create.summary.backupOff",
								"Off · you'll save a key backup file instead",
							)}
					<Change
						to={3}
						label={t("setup.create.changeBackup", "Change account backup")}
					/>
				</KvRow>
			</KeyValueList>
		</Block>
	);
}

/** R8, inline: what creating does, before the one button that does it. */
function BeforeRun() {
	const { t } = useTranslation("devices");
	const { draft, host, limits } = useSetup();
	const { maxPending, pending, maxPerDay } = limits;
	const hours = Math.round(limits.lifetimeS / 3600);
	const undo =
		maxPending !== undefined && pending !== undefined && maxPerDay !== undefined
			? t(
					"setup.create.before.undoSlots",
					"Cancel the setup any time before the device starts it. That frees the slot again. It uses 1 of your {{max, number}} unused-package slots ({{after, number}} of {{max, number}} after this) and 1 of today's {{cap, number}} packages.",
					{ max: maxPending, after: pending + 1, cap: maxPerDay },
				)
			: t(
					"setup.create.before.undo",
					"Cancel the setup any time before the device starts it. That frees the slot again.",
				);
	return (
		<div className="flex flex-col gap-1.5">
			<GroupLabel>
				{t("setup.create.before.label", "Before this runs")}
			</GroupLabel>
			<ConsequencePreview
				rows={{
					what: (
						<Trans
							t={t}
							i18nKey="setup.create.before.what"
							defaults="Makes owner keys for <1/> on this computer, registers the device with {{host}} and builds a setup package for {{platform}}."
							values={{
								host,
								platform: draft.target
									? enumLabel(t, "target", draft.target)
									: "",
							}}
							components={{ 1: <Mono>{draft.name}</Mono> }}
						/>
					),
					who: t(
						"setup.create.before.who",
						"Nobody else yet. The device appears in your list as a pending setup, and only you can see it.",
					),
					stays: t(
						"setup.create.before.stays",
						"Nothing runs anywhere until you start the package on the device.",
					),
					when: t(
						"setup.create.before.when",
						"Usually under a minute. The package then works for {{count, number}} h.",
						{ count: hours },
					),
					undo: { reversible: true, text: undo },
				}}
			/>
		</div>
	);
}

function ProgressChip({ failed }: Readonly<{ failed: boolean }>) {
	const { t } = useTranslation("devices");
	const { draft } = useSetup();
	if (failed)
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{t("setup.create.chip.failed", "Failed")}
			</StatusChip>
		);
	return draft.created ? (
		<StatusChip tone="good" icon={CircleCheck}>
			{t("setup.create.chip.ready", "Ready")}
		</StatusChip>
	) : (
		<StatusChip tone="info" icon={LoaderCircle} spin>
			{t("setup.create.chip.creating", "Creating")}
		</StatusChip>
	);
}

/** Why the run failed, in the viewer's words; the hub's or the client's own sentence sits behind "Details" (SPEC §6.3). */
function FailureResult({ failure }: Readonly<{ failure: SetupFailure }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { leaveLink } = useSetup();
	const technical = failure.detail;
	const toPending =
		failure.kind === "limit" || failure.kind === "cancel_failed";
	return (
		<>
			<InlineResult
				tone="critical"
				actions={
					toPending ? (
						<a
							{...leaveLink({ screen: "fleet", view: "devices" })}
							className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
						>
							{t("setup.pending.title", "Pending setups")}
						</a>
					) : undefined
				}
			>
				<span>{failureText(t, failure)}</span>
				{failure.retryAfterS === undefined ? null : (
					<span>
						{" "}
						{t("setup.create.failure.retryAt", "Try again after {{time}}.", {
							time: time.at(time.nowS + failure.retryAfterS),
						})}
					</span>
				)}
			</InlineResult>
			{technical ? <TechnicalDetails detail={technical} /> : null}
		</>
	);
}

/** The run: its five rows, and the outcome next to them (R9). */
function Progress() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, create } = useSetup();
	const phases = useCreatePhases();
	const { created } = draft;
	const failure =
		create.run.status === "failed" ? create.run.failure : undefined;
	const running = !failure && !created;
	return (
		<Block
			icon={Package}
			title={t("setup.create.progress.title", "Creating the package")}
			summary={<ProgressChip failed={!!failure} />}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("setup.create.progress.stamp", "in this window")}
				/>
			}
			foot={
				failure
					? t(
							"setup.create.progress.footFailed",
							"Trying again makes a new setup; it counts toward today's packages.",
						)
					: running
						? t(
								"setup.create.progress.foot",
								"Keep this window open while it runs. Progress is also in Activity.",
							)
						: undefined
			}
		>
			<Checklist
				items={phases}
				label={t("setup.create.progress.title", "Creating the package")}
			/>
			{failure ? <FailureResult failure={failure} /> : null}
			{created && !failure ? (
				<InlineResult tone="good">
					<Trans
						t={t}
						i18nKey="setup.create.progress.ready"
						defaults="Package ready. <1/> is registered and waits for its first start until {{until}}."
						values={{ until: time.at(created.expiresAt) }}
						components={{ 1: <Mono>{draft.name}</Mono> }}
					/>
				</InlineResult>
			) : null}
		</Block>
	);
}

/** What keeps Create from running, said above the summary: the password is gone, or the hub checks don't pass. */
function CreateGate() {
	const { t } = useTranslation("devices");
	const { secrets, checks, goTo } = useSetup();
	if (!secrets.password)
		return (
			<GateNotice
				kind="locked"
				title={t(
					"setup.create.gate.password",
					"Enter the device password again.",
				)}
				text={t(
					"setup.create.gate.passwordText",
					"It's never stored, so it's gone after the page reloaded. The app needs it to lock the new keys.",
				)}
				actions={
					<DvButton size="sm" icon={KeyRound} onClick={() => goTo(3)}>
						{t("setup.create.gate.toPassword", "Go to Password")}
					</DvButton>
				}
			/>
		);
	if (checks.pass) return null;
	return (
		<GateNotice
			kind="hub"
			title={t(
				"setup.create.gate.checks",
				"The hub checks don't pass right now.",
			)}
			text={t(
				"setup.create.gate.checksText",
				"Creating a package registers the device there, so it has to wait. Your choices are kept.",
			)}
			actions={
				<DvButton size="sm" icon={Server} onClick={() => goTo(0)}>
					{t("setup.platform.toCheck", "Go to Check")}
				</DvButton>
			}
		/>
	);
}

/** Step 4: review the choices, then make the keys, register the device and build the package. */
export function CreateStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { draft, create } = useSetup();
	const started = create.run.status !== "idle" || !!draft.created;
	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={5}
				total={STEP_COUNT}
				title={t("setup.create.title", "Create the package")}
				lede={
					started
						? undefined
						: t(
								"setup.create.lede",
								"Check your choices. Creating registers the device and can't be undone except by cancelling the setup.",
							)
				}
				className={started ? "sr-only" : undefined}
			/>
			{started ? (
				<Progress />
			) : (
				<>
					<CreateGate />
					<Summary />
					<BeforeRun />
				</>
			)}
		</>
	);
}
