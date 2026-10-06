"use client";

import { useTranslation } from "@flow-like/locales";
import { LockKeyhole } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../../ui/dialog";
import { Banner } from "../../primitives/banner";
import { DvButton } from "../../primitives/dv-button";
import { Field, SecretInput, SwitchField } from "../../primitives/form-fields";
import {
	type ModelUnlockBlock,
	type ModelUnlockFailure,
	type ModelUnlockPrompt,
	vaultDeviceName,
} from "./model-unlock";
import {
	type ModelUnlockBridge,
	usePromptAnswer,
	usePromptQueue,
	useVaultLookup,
} from "./use-model-unlock";

/** Mounted once in the desktop shell: asks for a device password when a run wants a model on a locked device. */
export function ModelUnlockPrompts({
	bridge,
}: Readonly<{ bridge: ModelUnlockBridge }>) {
	const { current, remove } = usePromptQueue(bridge);
	if (!current) return null;
	return (
		<UnlockPrompt
			key={current.id}
			bridge={bridge}
			prompt={current}
			done={remove}
		/>
	);
}

function UnlockPrompt({
	bridge,
	prompt,
	done,
}: Readonly<{
	bridge: ModelUnlockBridge;
	prompt: ModelUnlockPrompt;
	done: (promptId: string) => void;
}>) {
	const lookup = useVaultLookup(bridge, prompt.deviceId);
	const answer = usePromptAnswer(bridge, prompt, lookup, done);
	if (!lookup) return null;
	const deviceName =
		prompt.deviceName ??
		(lookup.kind === "ready" ? vaultDeviceName(lookup.vault) : undefined) ??
		prompt.deviceId;
	return (
		<ModelUnlockDialog
			prompt={prompt}
			deviceName={deviceName}
			block={lookup.kind === "blocked" ? lookup.block : undefined}
			busy={answer.busy}
			failure={answer.failure}
			onUnlock={answer.unlock}
			onDecline={answer.decline}
		/>
	);
}

export interface ModelUnlockDialogProps {
	prompt: ModelUnlockPrompt;
	deviceName: string;
	/** Only "Not now" is offered. */
	block?: ModelUnlockBlock;
	busy: boolean;
	failure?: ModelUnlockFailure;
	onUnlock(password: string, keepUnlocked: boolean): void;
	onDecline(): void;
}

export function ModelUnlockDialog({
	prompt,
	deviceName,
	block,
	busy,
	failure,
	onUnlock,
	onDecline,
}: Readonly<ModelUnlockDialogProps>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const [password, setPassword] = useState("");
	const [keep, setKeep] = useState(true);
	const ready = !block && !busy && password.length > 0;
	const submit = (event: FormEvent) => {
		event.preventDefault();
		if (ready) onUnlock(password, keep);
	};
	const dismiss = (open: boolean) => {
		if (!open && !busy) onDecline();
	};
	const names = {
		device: deviceName,
		model: prompt.modelName,
		run: prompt.runName ?? "",
	};
	return (
		<Dialog open onOpenChange={dismiss}>
			<DialogContent showCloseButton={false} className="sm:max-w-md">
				<form onSubmit={submit} className="flex flex-col gap-4">
					<DialogHeader>
						<DialogTitle className="flex items-center gap-2">
							<LockKeyhole aria-hidden className="size-4 shrink-0" />
							{t("models.unlockPrompt.title", "Unlock {{device}}?", names)}
						</DialogTitle>
						<DialogDescription>
							{prompt.runName
								? t(
										"models.unlockPrompt.askRun",
										"“{{run}}” wants to use {{model}}, which runs on {{device}}.",
										names,
									)
								: t(
										"models.unlockPrompt.askUnnamed",
										"A run wants to use {{model}}, which runs on {{device}}.",
										names,
									)}
						</DialogDescription>
					</DialogHeader>
					{block ? (
						<Banner tone="locked">
							<BlockText block={block} device={deviceName} />
						</Banner>
					) : (
						<>
							<Field
								id={`${id}-password`}
								label={t("models.unlockPrompt.password", "Device password")}
								hint={t(
									"models.unlockPrompt.passwordHint",
									"The keys open in this app only and never leave this computer.",
								)}
								error={
									failure ? (
										<FailureText failure={failure} device={deviceName} />
									) : undefined
								}
							>
								<SecretInput
									value={password}
									onValueChange={setPassword}
									disabled={busy}
								/>
							</Field>
							<SwitchField
								id={`${id}-keep`}
								checked={keep}
								onCheckedChange={setKeep}
								disabled={busy}
							>
								{t("models.unlockPrompt.keep", "Keep unlocked until I quit")}
								<span className="block text-xs text-muted-foreground">
									{t(
										"models.unlockPrompt.keepHint",
										"Runs reach it without asking until you quit Flow-Like. Off: it locks again after 30 min unused.",
									)}
								</span>
							</SwitchField>
						</>
					)}
					<p className="text-xs text-muted-foreground">
						{t(
							"models.unlockPrompt.timeout",
							"No answer within 2 minutes counts as Not now: the run goes on without this device.",
						)}
					</p>
					<DialogFooter>
						<DvButton onClick={onDecline} disabled={busy}>
							{t("models.unlockPrompt.decline", "Not now")}
						</DvButton>
						{block ? null : (
							<DvButton
								variant="primary"
								type="submit"
								busy={busy}
								aria-disabled={!ready}
							>
								{t("models.unlockPrompt.unlock", "Unlock")}
							</DvButton>
						)}
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function BlockText({
	block,
	device,
}: Readonly<{ block: ModelUnlockBlock; device: string }>) {
	const { t } = useTranslation("devices");
	switch (block) {
		case "no_vault":
			return t(
				"models.unlockPrompt.block.noVault",
				"This computer holds no keys for {{device}}. Add it in Devices first; until then runs go on without it.",
				{ device },
			);
		case "fresh_endpoint":
			return t(
				"models.unlockPrompt.block.freshEndpoint",
				"The keys for {{device}} were restored from a backup. Unlock it once in Devices; after that runs can ask here.",
				{ device },
			);
		case "signed_out":
			return t(
				"models.unlockPrompt.block.signedOut",
				"Sign in to unlock {{device}}.",
				{ device },
			);
	}
}

function FailureText({
	failure,
	device,
}: Readonly<{ failure: ModelUnlockFailure; device: string }>) {
	const { t } = useTranslation("devices");
	switch (failure) {
		case "wrong_password":
			return t(
				"models.unlockPrompt.failure.wrongPassword",
				"That password didn't open the keys. Try again.",
			);
		case "authority_mismatch":
			return t(
				"models.unlockPrompt.failure.authorityMismatch",
				"The signed identity of {{device}} doesn't match the keys on this computer. Check it in Devices.",
				{ device },
			);
		case "device_unavailable":
			return t(
				"models.unlockPrompt.failure.deviceUnavailable",
				"{{device}} was removed or is no longer shared with you.",
				{ device },
			);
		case "hub_unreachable":
			return t(
				"models.unlockPrompt.failure.hubUnreachable",
				"The hub couldn't be reached to check {{device}}. Check your connection and try again.",
				{ device },
			);
		case "signed_out":
			return t(
				"models.unlockPrompt.failure.signedOut",
				"Sign in again, then unlock.",
			);
		case "failed":
			return t(
				"models.unlockPrompt.failure.failed",
				"Unlocking failed. Try again.",
			);
	}
}
