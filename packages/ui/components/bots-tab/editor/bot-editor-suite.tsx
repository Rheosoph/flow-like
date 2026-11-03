"use client";

import { useTranslation } from "@flow-like/locales";
import { type ComponentType, useEffect } from "react";
import { useBotLabels } from "../../../hooks/use-bots";
import { useBackend, useSignedIn } from "../../../state/backend-state";
import type { IBot } from "../../../state/backend-state/bot-state";
import { ChatMemorySettings } from "../../global-chat/chat-memory-settings";
import { ChatModelSettings } from "../../global-chat/chat-model-settings";
import { DialogDescription, DialogTitle } from "../../ui/dialog";
import type { BotDraft } from "./bot-draft";
import { useBotPresetTexts } from "./bot-editor-copy";
import { type BotDiscardPrompt, BotEditorFooter } from "./bot-editor-footer";
import type { BotEditorMode, BotEditorStep } from "./bot-editor-steps";
import { findBotPreset } from "./bot-presets";
import { BotPreviewPanel } from "./bot-preview-panel";
import { BotStepNav } from "./bot-step-nav";
import { AbilitiesStep, FlowPilotAbilityStep } from "./steps/abilities-step";
import { BrainStep } from "./steps/brain-step";
import { DescribeStep } from "./steps/describe-step";
import { LocationStep } from "./steps/location-step";
import { LookStep } from "./steps/look-step";
import { BotMemoryModelField } from "./steps/memory-model-field";
import { SafetyStep } from "./steps/safety-step";
import { SecretsStep } from "./steps/secrets-step";
import { type BotEditor, useBotEditor } from "./use-bot-editor";
import { useBotEditorSubmit } from "./use-bot-editor-submit";

export interface BotEditorSuiteProps {
	mode: BotEditorMode;
	bot: IBot | null;
	onSaved?: (bot: IBot) => void;
	onDuplicated?: (bot: IBot) => void;
	/** Closes without asking; used after a save, a duplicate or a confirmed discard. */
	close: () => void;
	discard: BotDiscardPrompt;
	onDirtyChange: (dirty: boolean) => void;
}

type StepViewProps = Readonly<{ editor: BotEditor }>;

/** Signed out on the desktop every new bot is local, and a local bot stays local until it moves. */
function useLivesOnThisComputer(bot: IBot | null): boolean {
	const signedIn = useSignedIn();
	const keepsLocalBots = Boolean(useBackend().botState?.local);
	return bot?.local === true || (!signedIn && keepsLocalBots);
}

/** The whole creation suite: step navigation, a live preview beside each step, and the footer. */
export function BotEditorSuite({
	mode,
	bot,
	onSaved,
	onDuplicated,
	close,
	discard,
	onDirtyChange,
}: Readonly<BotEditorSuiteProps>) {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const localOnly = useLivesOnThisComputer(bot);
	const editor = useBotEditor(mode, bot, { localOnly });
	const submit = useBotEditorSubmit(editor, { onSaved, onDuplicated, close });
	useEffect(() => onDirtyChange(editor.dirty), [editor.dirty, onDirtyChange]);
	useEffect(() => () => onDirtyChange(false), [onDirtyChange]);
	return (
		<>
			<header className="flex shrink-0 flex-col gap-4 border-b border-border px-4 pt-4 pb-3 sm:px-6 sm:pt-5">
				<div className="flex min-w-0 flex-col gap-1 pr-8">
					<DialogTitle className="truncate">
						{editor.original
							? t("editor.title.edit")
							: t("editor.title.create")}
					</DialogTitle>
					<DialogDescription className="text-xs">
						{editor.original
							? t("editor.subtitle.edit", {
									name: labels(editor.original).name,
								})
							: t("editor.subtitle.create")}
					</DialogDescription>
				</div>
				<BotStepNav
					steps={editor.steps}
					current={editor.step}
					flagged={editor.flaggedSteps}
					onSelect={editor.goTo}
				/>
			</header>
			<div className="@container min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-5 sm:px-6 sm:py-6">
				<BotEditorStepBody editor={editor} />
			</div>
			<BotEditorFooter editor={editor} submit={submit} discard={discard} />
		</>
	);
}

/** A missing name is flagged once the user is about to create or save, not while they explore. */
function isNameInvalid(editor: BotEditor): boolean {
	const finishing =
		editor.mode === "edit" || editor.step === editor.steps.at(-1);
	for (const problem of editor.problems) {
		if (problem.code === "name_too_long") return true;
		if (problem.code === "name_required" && finishing) return true;
	}
	return false;
}

function usePreviewBadge(editor: BotEditor, shown: BotDraft) {
	const { t } = useTranslation("bots");
	const presetTexts = useBotPresetTexts();
	const preset = findBotPreset(shown.presetId);
	if (preset) return presetTexts(preset).name;
	return editor.step === "describe" ? t("editor.preview.blank") : undefined;
}

function BotEditorStepBody({ editor }: StepViewProps) {
	const system = editor.original?.system ?? null;
	const nameInvalid = isNameInvalid(editor);
	const describing = editor.step === "describe";
	const shown = describing ? editor.candidate : editor.draft;
	const badge = usePreviewBadge(editor, shown);
	if (editor.step === "look")
		return (
			<LookStep
				draft={editor.draft}
				system={system}
				nameInvalid={nameInvalid}
				onIdentityChange={editor.setIdentity}
				onAppearanceChange={editor.setAppearance}
			/>
		);
	const StepView = STEP_VIEWS[editor.step];
	return (
		<div className="grid items-start gap-7 @2xl:grid-cols-[minmax(220px,0.85fr)_minmax(0,2fr)]">
			<BotPreviewPanel
				draft={shown}
				system={system}
				readOnly={describing}
				badge={badge}
				nameInvalid={nameInvalid}
				onChange={editor.setIdentity}
				className="@2xl:sticky @2xl:top-0"
			/>
			<div className="@container min-w-0">
				<StepView editor={editor} />
			</div>
		</div>
	);
}

function DescribeView({ editor }: StepViewProps) {
	return (
		<DescribeStep
			text={editor.describeText}
			matches={editor.matches}
			choice={editor.choice}
			onTextChange={editor.setDescribeText}
			onPick={editor.pickPreset}
			onScratch={editor.startFromScratch}
		/>
	);
}

function AbilitiesView({ editor }: StepViewProps) {
	if (editor.flowPilot) return <FlowPilotAbilityStep />;
	return (
		<AbilitiesStep
			draft={editor.draft}
			botId={editor.original?.id ?? null}
			pendingAppIds={editor.pendingAppIds}
			onPendingAppIdsChange={editor.setPendingAppIds}
			onToggle={editor.toggleCapability}
			onProfileChange={editor.setProfileId}
			onEmbeddingModelChange={editor.setEmbeddingModel}
		/>
	);
}

/**
 * Flux's saved memories are FlowPilot's, per profile; every other bot, Flux's copies included, keeps
 * its own. A FlowPilot bot always remembers, so its memory model sits here instead of in a Memory card.
 */
function BrainView({ editor }: StepViewProps) {
	const { draft, original } = editor;
	return (
		<div className="flex flex-col gap-5">
			<BrainStep
				instructions={draft.instructions}
				model={draft.model}
				location={draft.location}
				onInstructionsChange={editor.setInstructions}
				onModelChange={editor.setModel}
			>
				{!draft.model.primary && <ChatModelSettings />}
				{editor.flowPilot && (
					<BotMemoryModelField
						value={draft.embeddingModel}
						onChange={editor.setEmbeddingModel}
					/>
				)}
			</BrainStep>
			{original && (
				<ChatMemorySettings
					botId={original.system === "flux" ? null : original.id}
				/>
			)}
		</div>
	);
}

function LocationView({ editor }: StepViewProps) {
	return (
		<LocationStep
			location={editor.draft.location}
			localOnly={editor.localOnly}
			deviceId={editor.draft.deviceId}
			removed={editor.removedCapabilities}
			restored={editor.restoredCapabilities}
			onLocationChange={editor.setLocation}
			onDeviceChange={editor.setDeviceId}
		/>
	);
}

function SafetyView({ editor }: StepViewProps) {
	return (
		<SafetyStep
			draft={editor.draft}
			locked={editor.flowPilot}
			onClassificationChange={editor.setClassification}
			onToggle={editor.toggleCapability}
		/>
	);
}

function SecretsView({ editor }: StepViewProps) {
	return editor.original ? <SecretsStep botId={editor.original.id} /> : null;
}

const STEP_VIEWS: Readonly<
	Record<Exclude<BotEditorStep, "look">, ComponentType<StepViewProps>>
> = {
	describe: DescribeView,
	abilities: AbilitiesView,
	brain: BrainView,
	location: LocationView,
	safety: SafetyView,
	secrets: SecretsView,
};
