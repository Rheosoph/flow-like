"use client";

import { isEqual } from "lodash-es";
import { useCallback, useMemo, useState } from "react";
import type { IDataSensitivity } from "../../../lib/data-classification";
import type {
	BotCapability,
	BotLocation,
	IBot,
	IBotModelPolicy,
	IUpdateBotRequest,
} from "../../../state/backend-state/bot-state";
import type { BotAppearance } from "../../bots/types";
import {
	type BotDraft,
	type BotDraftProblem,
	type BotLocationMove,
	type BotModelPin,
	botDraftFromBot,
	botDraftFromPreset,
	buildUpdateBotRequest,
	createEmptyBotDraft,
	getBotDraftProblems,
	hasFixedLocation,
	hasFlowPilot,
	keepOnThisComputer,
	setBotDraftCapability,
	setBotDraftLocation,
} from "./bot-draft";
import { useBotPresetTexts } from "./bot-editor-copy";
import {
	BOT_SCRATCH_CHOICE,
	type BotEditorMode,
	type BotEditorStep,
	getBotEditorSteps,
	getBotProblemStep,
} from "./bot-editor-steps";
import { type BotPresetMatch, rankBotPresets } from "./bot-preset-match";
import { type BotPreset, findBotPreset } from "./bot-presets";
import type { BotIdentityChange } from "./bot-preview-panel";

type DraftSetter = (draft: BotDraft | ((draft: BotDraft) => BotDraft)) => void;

export interface BotEditor {
	mode: BotEditorMode;
	/** The bot as it was when the editor opened; edits are measured against it. */
	original: IBot | null;
	/** Flux or one of its duplicates: FlowPilot's complete toolset, so its abilities are fixed. */
	flowPilot: boolean;
	/** Flux: where it runs can't be changed either. */
	fixedLocation: boolean;
	/** The bot lives only on this computer, so it can't run anywhere else. */
	localOnly: boolean;
	steps: BotEditorStep[];
	step: BotEditorStep;
	draft: BotDraft;
	/** What Describe previews before anything is applied: the best match, else the draft. */
	candidate: BotDraft;
	describeText: string;
	matches: BotPresetMatch[];
	choice: string | null;
	pendingAppIds: string[];
	/** Switched off by the last move, because the new host can't do them. */
	removedCapabilities: BotCapability[];
	/** Switched back on by the last move, after an earlier move had switched them off. */
	restoredCapabilities: BotCapability[];
	problems: BotDraftProblem[];
	flaggedSteps: ReadonlySet<BotEditorStep>;
	updateRequest: IUpdateBotRequest | null;
	dirty: boolean;
	goTo: (step: BotEditorStep) => void;
	setDescribeText: (text: string) => void;
	pickPreset: (preset: BotPreset) => void;
	startFromScratch: () => void;
	setIdentity: (change: BotIdentityChange) => void;
	setAppearance: (appearance: BotAppearance) => void;
	toggleCapability: (capability: BotCapability, enabled: boolean) => void;
	setProfileId: (profileId: string) => void;
	setPendingAppIds: (ids: string[]) => void;
	setInstructions: (instructions: string) => void;
	setModel: (model: IBotModelPolicy) => void;
	setEmbeddingModel: (embeddingModel: string | null) => void;
	setModelPin: (pin: BotModelPin, bitId: string | null) => void;
	setLocation: (location: BotLocation) => void;
	setDeviceId: (deviceId: string) => void;
	setClassification: (classification: IDataSensitivity) => void;
}

/** The free-text description, the presets it matches and what was applied from them. */
function useDescribe(setDraft: DraftSetter) {
	const presetTexts = useBotPresetTexts();
	const [text, setText] = useState("");
	const [choice, setChoice] = useState<string | null>(null);
	const [descriptionTouched, setDescriptionTouched] = useState(false);
	const matches = useMemo(() => rankBotPresets(text), [text]);

	const draftFor = useCallback(
		(preset: BotPreset) => {
			const texts = presetTexts(preset);
			return botDraftFromPreset(preset, {
				...texts,
				description: text.trim() || texts.description,
			});
		},
		[presetTexts, text],
	);

	const pick = useCallback(
		(preset: BotPreset) => {
			setDraft(draftFor(preset));
			setChoice(preset.id);
			setDescriptionTouched(false);
		},
		[draftFor, setDraft],
	);

	const scratch = useCallback(() => {
		setDraft({ ...createEmptyBotDraft(), description: text.trim() });
		setChoice(BOT_SCRATCH_CHOICE);
		setDescriptionTouched(false);
	}, [setDraft, text]);

	/** Leaving Describe takes the best match unless something was picked; newer text still becomes the description. */
	const settle = useCallback(() => {
		if (choice === null) {
			const top = matches[0]?.preset;
			if (top) pick(top);
			else scratch();
			return;
		}
		if (descriptionTouched) return;
		const preset = findBotPreset(choice);
		const description =
			text.trim() || (preset ? presetTexts(preset).description : "");
		setDraft((draft) => ({ ...draft, description }));
	}, [
		choice,
		descriptionTouched,
		matches,
		pick,
		presetTexts,
		scratch,
		setDraft,
		text,
	]);

	const touchDescription = useCallback(() => setDescriptionTouched(true), []);

	return {
		text,
		setText,
		choice,
		matches,
		pick,
		scratch,
		settle,
		draftFor,
		touchDescription,
	};
}

type BotLocationChange = Omit<BotLocationMove, "draft">;

const NO_LOCATION_CHANGE: BotLocationChange = {
	removed: [],
	restored: [],
	remembered: [],
};

/** A capability the user switches off by hand stays off, wherever the bot moves next. */
function forgetCapability(
	change: BotLocationChange,
	capability: BotCapability,
): BotLocationChange {
	const without = (capabilities: BotCapability[]) =>
		capabilities.filter((entry) => entry !== capability);
	return {
		removed: without(change.removed),
		restored: without(change.restored),
		remembered: without(change.remembered),
	};
}

function isDirty(
	original: IBot | null,
	updateRequest: IUpdateBotRequest | null,
	draft: BotDraft,
	untouched: boolean,
	empty: BotDraft,
): boolean {
	if (original) return Object.keys(updateRequest ?? {}).length > 0;
	return !untouched || !isEqual(draft, empty);
}

const placeAnywhere = (draft: BotDraft) => draft;

export function useBotEditor(
	mode: BotEditorMode,
	bot: IBot | null,
	options: { localOnly?: boolean } = {},
): BotEditor {
	const localOnly = options.localOnly === true;
	const place = localOnly ? keepOnThisComputer : placeAnywhere;
	const [original] = useState(bot);
	const system = original?.system ?? null;
	const flowPilot = original ? hasFlowPilot(original) : false;
	const fixedLocation = hasFixedLocation(system);
	const steps = useMemo(
		() => getBotEditorSteps(mode, { fixedLocation }),
		[mode, fixedLocation],
	);
	const [step, setStep] = useState<BotEditorStep>(steps[0]);
	const [draft, storeDraft] = useState<BotDraft>(() =>
		place(original ? botDraftFromBot(original) : createEmptyBotDraft()),
	);
	const setDraft = useCallback<DraftSetter>(
		(next) =>
			storeDraft((current) =>
				place(typeof next === "function" ? next(current) : next),
			),
		[place],
	);
	const [pendingAppIds, setPendingAppIds] = useState<string[]>([]);
	const [locationChange, setLocationChange] =
		useState<BotLocationChange>(NO_LOCATION_CHANGE);
	const {
		text,
		choice,
		matches,
		draftFor,
		settle,
		pick,
		scratch,
		...describe
	} = useDescribe(setDraft);

	const problems = useMemo(
		() =>
			getBotDraftProblems(draft, {
				system,
				checkLook: !original || !isEqual(draft.appearance, original.appearance),
			}),
		[draft, original, system],
	);
	const flaggedSteps = useMemo(
		() =>
			new Set(problems.flatMap((problem) => getBotProblemStep(problem) ?? [])),
		[problems],
	);
	const updateRequest = useMemo(
		() => (original ? buildUpdateBotRequest(original, draft) : null),
		[original, draft],
	);
	const untouched =
		!text.trim() && choice === null && pendingAppIds.length === 0;
	const candidate = useMemo(() => {
		const top = matches[0]?.preset;
		return choice === null && top ? place(draftFor(top)) : draft;
	}, [choice, draft, draftFor, matches, place]);

	const update = useCallback(
		(patch: Partial<BotDraft>) =>
			setDraft((current) => ({ ...current, ...patch })),
		[setDraft],
	);

	const goTo = useCallback(
		(next: BotEditorStep) => {
			if (step === "describe" && next !== "describe") settle();
			setStep(next);
		},
		[settle, step],
	);

	return {
		mode,
		original,
		flowPilot,
		fixedLocation,
		localOnly,
		steps,
		step,
		draft,
		candidate,
		describeText: text,
		matches,
		choice,
		pendingAppIds,
		removedCapabilities: locationChange.removed,
		restoredCapabilities: locationChange.restored,
		problems,
		flaggedSteps,
		updateRequest,
		dirty: isDirty(
			original,
			updateRequest,
			draft,
			untouched,
			place(createEmptyBotDraft()),
		),
		goTo,
		setDescribeText: describe.setText,
		pickPreset: (preset) => {
			if (preset.id === choice) return;
			pick(preset);
			setLocationChange(NO_LOCATION_CHANGE);
		},
		startFromScratch: () => {
			scratch();
			setLocationChange(NO_LOCATION_CHANGE);
			setStep("look");
		},
		setIdentity: (change) => {
			if (change.description !== undefined) describe.touchDescription();
			update(change);
		},
		setAppearance: (appearance) => update({ appearance }),
		toggleCapability: (capability, enabled) => {
			if (flowPilot) return;
			setDraft((current) =>
				setBotDraftCapability(current, capability, enabled),
			);
			if (!enabled)
				setLocationChange((current) => forgetCapability(current, capability));
		},
		setProfileId: (profileId) => update({ profileId }),
		setPendingAppIds,
		setInstructions: (instructions) => update({ instructions }),
		setModel: (model) => update({ model }),
		setEmbeddingModel: (embeddingModel) => update({ embeddingModel }),
		setModelPin: (pin, bitId) => update({ [pin]: bitId }),
		setLocation: (location) => {
			if (localOnly || fixedLocation) return;
			if (flowPilot) return update({ location });
			const { draft: moved, ...change } = setBotDraftLocation(
				draft,
				location,
				locationChange.remembered,
			);
			setDraft(moved);
			setLocationChange(change);
		},
		setDeviceId: (deviceId) => update({ deviceId }),
		setClassification: (classification) => update({ classification }),
	};
}
