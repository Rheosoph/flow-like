"use client";

import { useTranslation } from "@flow-like/locales";
import type { DeviceErrorCode } from "../../../../lib/device-management/workspace/errors";
import type {
	StepDetailCode,
	UnlockStep,
	UnlockStepId,
} from "../../../../lib/device-management/workspace/types";
import { errorCopy } from "../copy/error-copy";
import type { DevicesT } from "../primitives/area-context";
import {
	type CheckState,
	Checklist,
	type ChecklistItem,
} from "../primitives/checklist";

const KEY_STEPS: readonly UnlockStepId[] = [
	"unlocking_keys",
	"checking_identity",
	"rotating_endpoint",
	"saving_backup",
];
const LIVE_STEPS: readonly UnlockStepId[] = [
	"getting_pass",
	"reaching_device",
	"trying_direct",
	"securing",
	"reading_services",
];
const STATUS_STEP: UnlockStepId = "reading_encrypted_status";

/** Steps that only appear once they ran: most unlocks neither rotate the connection identity nor save a backup. */
const OPTIONAL_STEPS = new Set<UnlockStepId>([
	"rotating_endpoint",
	"saving_backup",
]);
/** The keys are open by then, so a failure there is a warning, not a failed unlock. */
const SOFT_STEPS = new Set<UnlockStepId>(["saving_backup", STATUS_STEP]);

const pending = (id: UnlockStepId): UnlockStep => ({ id, state: "pending" });

/**
 * The lines shown after the password was entered (IA §6.4.3): the key steps
 * the unlock reported, then the live connection's steps or, without "Connect
 * live", the encrypted status read.
 */
export function connectionSteps(
	keySteps: readonly UnlockStep[],
	liveSteps: readonly UnlockStep[],
	connectLive: boolean,
): UnlockStep[] {
	const rows = keyRows(keySteps);
	if (connectLive) return [...rows, ...liveRows(keySteps, liveSteps)];
	return [...rows, stepOf(keySteps, STATUS_STEP) ?? pending(STATUS_STEP)];
}

function stepOf(steps: readonly UnlockStep[], id: UnlockStepId) {
	return steps.find((step) => step.id === id);
}

function keyRows(keySteps: readonly UnlockStep[]): UnlockStep[] {
	return KEY_STEPS.flatMap((id) => {
		const step = stepOf(keySteps, id);
		const idle = !step || step.state === "skipped" || step.state === "pending";
		return OPTIONAL_STEPS.has(id) && idle ? [] : [step ?? pending(id)];
	});
}

/** The connection's own steps count only once this unlock opened the keys: before that they belong to an earlier attempt. */
function liveRows(
	keySteps: readonly UnlockStep[],
	liveSteps: readonly UnlockStep[],
): UnlockStep[] {
	const unlocked = stepOf(keySteps, "checking_identity")?.state === "done";
	const source = unlocked ? liveSteps : [];
	return LIVE_STEPS.map((id) => stepOf(source, id) ?? pending(id));
}

interface StepWords {
	/** While it waits or runs. */
	name: string;
	done: string;
}

function stepWords(t: DevicesT, id: UnlockStepId): StepWords {
	const words: Record<UnlockStepId, StepWords> = {
		unlocking_keys: {
			name: t("devices:overlay.progress.unlockingKeys", "Unlocking keys"),
			done: t("devices:overlay.progress.unlockingKeysDone", "Keys unlocked"),
		},
		checking_identity: {
			name: t(
				"devices:overlay.progress.checkingIdentity",
				"Checking device identity",
			),
			done: t(
				"devices:overlay.progress.checkingIdentityDone",
				"Device identity matches",
			),
		},
		rotating_endpoint: {
			name: t(
				"devices:overlay.progress.rotatingEndpoint",
				"Renewing this computer's connection identity",
			),
			done: t(
				"devices:overlay.progress.rotatingEndpointDone",
				"This computer got a new connection identity. Encrypted metric groups will need the owner's re-approval.",
			),
		},
		saving_backup: {
			name: t(
				"devices:overlay.progress.savingBackup",
				"Saving to your account backup",
			),
			done: t(
				"devices:overlay.progress.savingBackupDone",
				"Saved to your account backup",
			),
		},
		reading_encrypted_status: {
			name: t(
				"devices:overlay.progress.readingStatus",
				"Reading encrypted status",
			),
			done: t(
				"devices:overlay.progress.readingStatusDone",
				"Encrypted status read",
			),
		},
		getting_pass: {
			name: t(
				"devices:overlay.progress.gettingPass",
				"Getting a connection pass",
			),
			done: t(
				"devices:overlay.progress.gettingPassDone",
				"Connection pass received",
			),
		},
		reaching_device: {
			name: t(
				"devices:overlay.progress.reachingDevice",
				"Reaching the device through the hub",
			),
			done: t("devices:overlay.progress.reachingDeviceDone", "Device answered"),
		},
		trying_direct: {
			name: t(
				"devices:overlay.progress.tryingDirect",
				"Trying a direct connection",
			),
			done: t("devices:overlay.progress.tryingDirectDone", "Direct connection"),
		},
		securing: {
			name: t("devices:overlay.progress.securing", "Securing the connection"),
			done: t("devices:overlay.progress.securingDone", "Connection secured"),
		},
		reading_services: {
			name: t("devices:overlay.progress.readingServices", "Reading services"),
			done: t("devices:overlay.progress.readingServicesDone", "Services read"),
		},
	};
	return words[id];
}

type OwnDetail =
	| "fresh_endpoint"
	| "backup_saved"
	| "backup_local_only"
	| "backup_limit"
	| "not_requested";

function ownDetail(t: DevicesT, code: OwnDetail): string | undefined {
	const sentences: Record<OwnDetail, string | undefined> = {
		fresh_endpoint: undefined,
		backup_saved: undefined,
		not_requested: undefined,
		backup_local_only: t(
			"devices:overlay.progress.backupLocalOnly",
			"The backup couldn't be uploaded. It stays on this computer and uploads when the hub is reachable.",
		),
		backup_limit: t(
			"devices:overlay.progress.backupLimit",
			"Your account has no free backup slot, so the backup stays on this computer only.",
		),
	};
	return sentences[code];
}

const OWN_DETAILS = new Set<StepDetailCode>([
	"fresh_endpoint",
	"backup_saved",
	"backup_local_only",
	"backup_limit",
	"not_requested",
]);

/** The plain sentence of a step's detail code (IA §6.4.3: every error maps to a step and a sentence). */
function detailSentence(t: DevicesT, step: UnlockStep): string | undefined {
	const detail = step.detail;
	if (!detail) return undefined;
	if (OWN_DETAILS.has(detail.code))
		return ownDetail(t, detail.code as OwnDetail);
	return errorCopy(t, detail.code as DeviceErrorCode, { ...detail.params });
}

function failedText(t: DevicesT, step: UnlockStep, words: StepWords): string {
	if (step.id === STATUS_STEP && !step.detail)
		return t(
			"devices:overlay.progress.readingStatusFailed",
			"The encrypted status couldn't be read yet. It's retried in the background.",
		);
	return (
		detailSentence(t, step) ??
		t("devices:overlay.progress.failed", "{{step}} didn't finish.", {
			step: words.name,
		})
	);
}

interface StepLook {
	state: CheckState;
	text: string;
}

function stepLook(t: DevicesT, step: UnlockStep): StepLook {
	const words = stepWords(t, step.id);
	switch (step.state) {
		case "pending":
			return { state: "pending", text: words.name };
		case "active":
			return { state: "active", text: words.name };
		case "done":
			return { state: "pass", text: words.done };
		case "failed":
			return {
				state: SOFT_STEPS.has(step.id) ? "warn" : "fail",
				text: failedText(t, step, words),
			};
		case "skipped": {
			const sentence = detailSentence(t, step);
			return sentence
				? { state: "warn", text: sentence }
				: { state: "skip", text: words.name };
		}
	}
}

/** Connection progress, one line per step with its sentence (IA §6.4.3). */
export function ConnectionProgress({
	steps,
	className,
}: Readonly<{ steps: readonly UnlockStep[]; className?: string }>) {
	const { t } = useTranslation("devices");
	const items: ChecklistItem[] = steps.map((step) => {
		const look = stepLook(t, step);
		return { id: step.id, state: look.state, label: look.text };
	});
	return (
		<Checklist
			items={items}
			label={t("overlay.progress.label", "Unlock progress")}
			className={className}
		/>
	);
}

/** Copy diagnostics lines: English and machine-oriented on purpose (R3), never translated. */
export function progressReport(steps: readonly UnlockStep[]): string[] {
	return steps.map(
		(step) =>
			`${step.id} ${step.state}${step.detail ? ` ${step.detail.code}` : ""}`,
	);
}
