"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Package } from "lucide-react";
import type { ReactNode } from "react";
import { useAreaTime } from "../primitives/area-context";
import type { CheckState, ChecklistItem } from "../primitives/checklist";
import { IdRef } from "../primitives/id-ref";
import { ProgressBar } from "../primitives/meter";
import { useSetup } from "./setup-context";
import { Mono } from "./setup-parts";
import { type SetupDraft, packageFile, packsAgent } from "./setup-state";
import { megabytes } from "./steps/platform-step";
import type { CreatePhase, CreateRun } from "./use-setup-create";

/* Step 4's five rows: what each phase will do, and what it did (SPEC §5.5). */

function phaseState(
	run: CreateRun,
	phase: CreatePhase,
	finished: boolean,
): CheckState {
	if (finished || run.doneAt[phase] !== undefined) return "pass";
	if (run.phase !== phase) return "pending";
	return run.status === "failed" ? "fail" : "active";
}

/** An account backup that was not saved is a warning, not a failed creation. */
function backupState(draft: SetupDraft, state: CheckState): CheckState {
	if (!draft.backup) return "skip";
	const outcome = draft.created?.outcome;
	const unsaved = outcome === "local_only" || outcome === "limit";
	return state === "pass" && unsaved ? "warn" : state;
}

function hostOf(url: string | undefined): string {
	if (!url) return "";
	try {
		return new URL(url).host;
	} catch {
		return "";
	}
}

/** A finished phase's note, followed by when this window finished it. */
function Timed({
	phase,
	children,
}: Readonly<{ phase: CreatePhase; children: ReactNode }>) {
	const time = useAreaTime();
	const ended = useSetup().create.run.doneAt[phase];
	return (
		<>
			{children}
			{ended === undefined ? null : (
				<> · {time.clock(Math.floor(ended / 1000))}</>
			)}
		</>
	);
}

function KeysNote({ done }: Readonly<{ done: boolean }>) {
	const { t } = useTranslation("devices");
	return done ? (
		<Timed phase="keys">
			{t(
				"setup.create.phase.keysDone",
				"Owner keys made on this computer and locked with the device password",
			)}
		</Timed>
	) : (
		t(
			"setup.create.phase.keysTodo",
			"Makes the keys only this computer holds. Takes a few seconds.",
		)
	);
}

/** The two ids the hub answered with, once the device is registered. */
function RegisteredIds() {
	const { t } = useTranslation("devices");
	const { draft, create } = useSetup();
	const ids = create.run.registration ?? draft.created;
	if (!ids?.deviceId || !ids.enrollmentId) return null;
	return (
		<>
			{" · "}
			<IdRef
				id={ids.deviceId}
				label={t("setup.field.deviceId", "Device ID")}
				copyLabel={t("setup.copy.deviceId", "Copy device ID")}
			/>{" "}
			<IdRef
				id={ids.enrollmentId}
				label={t("setup.field.setupId", "Setup ID")}
				copyLabel={t("setup.copy.setupId", "Copy setup ID")}
			/>
		</>
	);
}

function RegisterNote({ done }: Readonly<{ done: boolean }>) {
	const { t } = useTranslation("devices");
	const { limits } = useSetup();
	const hours = Math.round(limits.lifetimeS / 3600);
	if (done)
		return (
			<>
				<Timed phase="register">
					{t("setup.create.phase.registerDone", "Registered")}
				</Timed>
				<RegisteredIds />
			</>
		);
	return limits.maxPending === undefined
		? t(
				"setup.create.phase.registerTodoPlain",
				"Reserves one of your unused-package slots for up to {{count, number}} h.",
				{ count: hours },
			)
		: t(
				"setup.create.phase.registerTodo",
				"Reserves 1 of your {{max, number}} unused-package slots for up to {{count, number}} h.",
				{ max: limits.maxPending, count: hours },
			);
}

/** Why this window does not download the agent: Docker pulls an image, or the device fetches a large one itself. */
function DownloadSkipped() {
	const { t } = useTranslation("devices");
	const { draft, option } = useSetup();
	return draft.mode === "docker"
		? t(
				"setup.create.phase.downloadDocker",
				"Docker pulls the pinned image on the device instead.",
			)
		: t(
				"setup.create.phase.downloadDeferred",
				"The device downloads it on first start ({{size}}, larger than 256 MiB).",
				{ size: megabytes(option?.size ?? 0) },
			);
}

function DownloadNote({ state }: Readonly<{ state: CheckState }>) {
	const { t } = useTranslation("devices");
	const { draft, option, release } = useSetup();
	if (state === "skip") return <DownloadSkipped />;
	const size = megabytes(option?.size ?? 0);
	if (state === "pass")
		return (
			<Timed phase="download">
				{t(
					"setup.create.phase.downloadDone",
					"{{size}} · matches the signed release",
					{ size },
				)}
			</Timed>
		);
	const artifact = release?.manifest.artifacts.find(
		(item) => item.target === draft.target,
	);
	return t(
		"setup.create.phase.downloadTodo",
		"{{size}} from {{host}}. Checked against the signed release when it finishes.",
		{ size, host: hostOf(artifact?.url) },
	);
}

function BuildNote({ done }: Readonly<{ done: boolean }>) {
	const { t } = useTranslation("devices");
	const { draft } = useSetup();
	return done ? (
		<Timed phase="build">
			<Trans
				t={t}
				i18nKey="setup.create.phase.buildDone"
				defaults="<1/> · includes a one-time setup secret"
				components={{ 1: <Mono>{packageFile(draft.name)}</Mono> }}
			/>
		</Timed>
	) : (
		t(
			"setup.create.phase.buildTodo",
			"Adds the start scripts, the signed release and a one-time setup secret.",
		)
	);
}

function BackupLabel({ state }: Readonly<{ state: CheckState }>) {
	const { t } = useTranslation("devices");
	const outcome = useSetup().draft.created?.outcome;
	if (state === "skip")
		return t(
			"setup.create.phase.backupSkipped",
			"Saving account backup · skipped",
		);
	if (state !== "warn")
		return t("setup.create.phase.backup", "Saving account backup");
	return outcome === "limit"
		? t(
				"setup.create.phase.backupLimit",
				"Saving account backup · limit reached",
			)
		: t(
				"setup.create.phase.backupLocal",
				"Saving account backup · didn't finish",
			);
}

function BackupUnsaved() {
	const { t } = useTranslation("devices");
	const outcome = useSetup().draft.created?.outcome;
	return outcome === "limit"
		? t(
				"setup.create.phase.backupLimitNote",
				"The hub refused another account backup. Save the key backup file next.",
			)
		: t(
				"setup.create.phase.backupLocalNote",
				"The keys are only on this computer. Save the key backup file next.",
			);
}

function BackupNote({ state }: Readonly<{ state: CheckState }>) {
	const { t } = useTranslation("devices");
	if (state === "skip")
		return t(
			"setup.create.phase.backupOff",
			"Account backup is off. Save the key backup file in the next step.",
		);
	if (state === "warn") return <BackupUnsaved />;
	return state === "pass" ? (
		<Timed phase="backup">
			{t("setup.create.phase.backupDone", "Saved to your account")}
		</Timed>
	) : (
		t(
			"setup.create.phase.backupTodo",
			"Encrypted with the device password before it leaves this computer.",
		)
	);
}

const strong = (label: ReactNode) => <b className="font-medium">{label}</b>;

/** Preparing keys → Registering with the hub → Downloading the agent → Building the package → Saving account backup. */
export function useCreatePhases(): ChecklistItem[] {
	const { t } = useTranslation("devices");
	const { draft, create, option } = useSetup();
	const finished = !!draft.created;
	const stateOf = (phase: CreatePhase) =>
		phaseState(create.run, phase, finished);
	const packs = packsAgent(option, draft.mode);
	const download = packs ? stateOf("download") : "skip";
	const backup = backupState(draft, stateOf("backup"));
	return [
		{
			id: "keys",
			state: stateOf("keys"),
			label: strong(t("setup.create.phase.keys", "Preparing keys")),
			note: <KeysNote done={stateOf("keys") === "pass"} />,
		},
		{
			id: "register",
			state: stateOf("register"),
			source: "hub",
			label: strong(
				t("setup.create.phase.register", "Registering with the hub"),
			),
			note: <RegisterNote done={stateOf("register") === "pass"} />,
		},
		{
			id: "download",
			state: download,
			source: {
				icon: Package,
				label: t("setup.create.phase.releaseServer", "Release server"),
			},
			label: strong(
				packs
					? t("setup.create.phase.download", "Downloading the agent")
					: t(
							"setup.create.phase.downloadSkipped",
							"Downloading the agent · skipped",
						),
			),
			note: <DownloadNote state={download} />,
			fix:
				download === "active" ? (
					<ProgressBar
						label={t("setup.create.phase.downloadProgress", "Agent download")}
						className="max-w-80 flex-1"
					/>
				) : undefined,
		},
		{
			id: "build",
			state: stateOf("build"),
			label: strong(t("setup.create.phase.build", "Building the package")),
			note: <BuildNote done={stateOf("build") === "pass"} />,
		},
		{
			id: "backup",
			state: backup,
			source: "hub",
			label: strong(<BackupLabel state={backup} />),
			note: <BackupNote state={backup} />,
		},
	];
}
