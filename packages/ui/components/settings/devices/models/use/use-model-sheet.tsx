"use client";

import { useTranslation } from "@flow-like/locales";
import { Plug } from "lucide-react";
import { useId, useState } from "react";
import { useInvalidateInvoke } from "../../../../../hooks/use-invoke";
import { deviceLabel } from "../../../../../lib/device-management/model/device-view";
import type { HostedModel } from "../../../../../lib/device-management/models";
import { asArray } from "../../../../../lib/response-shape";
import type { IBit } from "../../../../../lib/schema/bit/bit";
import {
	type IBackendState,
	useBackend,
} from "../../../../../state/backend-state";
import { gateView } from "../../device/use-device-page";
import { KeepForModels } from "../../overlays/unlock-sheet";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { DvInput, Field, InputWithUnit } from "../../primitives/form-fields";
import { GatedAction } from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import {
	useAttentionState,
	useDeviceWorkspace,
	useGate,
	useKeyChip,
	useKeySession,
} from "../../workspace";
import { kindLabel } from "../models-copy";
import {
	BIT_NAME_MAX,
	DeviceBitError,
	type DeviceBitFailure,
	MAX_CONTEXT,
	MAX_VECTOR_LENGTH,
	defaultContext,
	deviceBitIdFor,
	deviceModelBit,
	saveDeviceModelBit,
	wholeNumber,
} from "./device-bit";

type UsedModel = Pick<HostedModel, "id" | "display_name" | "kind" | "settings">;

type Outcome =
	| { kind: "saved"; name: string; inProfile: boolean }
	| { kind: "failed"; failure: DeviceBitFailure };

interface Draft {
	name: string;
	context: string;
	vector: string;
	keep: boolean;
}

interface Problems {
	name?: string;
	context?: string;
	vector?: string;
}

function problemsOf(t: DevicesT, model: UsedModel, draft: Draft): Problems {
	const name = draft.name.trim();
	const problems: Problems = {};
	if (!name || name.length > BIT_NAME_MAX)
		problems.name = t(
			"devices:models.use.problem.name",
			"Give it a name of up to {{max, number}} characters.",
			{ max: BIT_NAME_MAX },
		);
	if (wholeNumber(draft.context, MAX_CONTEXT) === undefined)
		problems.context = t(
			"devices:models.use.problem.context",
			"Enter the tokens per request as a whole number.",
		);
	if (
		model.kind === "embedding" &&
		wholeNumber(draft.vector, MAX_VECTOR_LENGTH) === undefined
	)
		problems.vector = t(
			"devices:models.use.problem.vector",
			"Enter how many numbers the model returns per text, e.g. 768.",
		);
	return problems;
}

function failureText(
	t: DevicesT,
	failure: DeviceBitFailure,
	device: string,
): string {
	const copy: Record<DeviceBitFailure, () => string> = {
		hub_refused: () =>
			t(
				"devices:models.use.result.hubRefused",
				"This hub doesn't take models from devices yet, so nothing was added. Add it from the Flow-Like desktop app, or once the hub is updated.",
			),
		not_allowed: () =>
			t(
				"devices:models.use.result.notAllowed",
				"The hub says your account can't use the models of {{device}}. Ask its owner for Use models.",
				{ device },
			),
		failed: () =>
			t(
				"devices:models.use.result.failed",
				"The model wasn't added to your models. Try again.",
			),
	};
	return copy[failure]();
}

/**
 * Where the saved Bit can be picked. A browser can't reach devices, so its
 * model pickers leave device Bits out; only the desktop app offers them.
 */
function savedText(
	t: DevicesT,
	name: string,
	inProfile: boolean,
	desktop: boolean,
): string {
	if (desktop)
		return inProfile
			? t(
					"devices:models.use.result.saved",
					"“{{name}}” is in your models. Pick it in any model picker.",
					{ name },
				)
			: t(
					"devices:models.use.result.savedNotActive",
					"“{{name}}” is in your models, but not on in the profile you use. Turn it on in Settings › Models.",
					{ name },
				);
	return inProfile
		? t(
				"devices:models.use.result.savedWeb",
				"“{{name}}” is in your models. Pick it in the Flow-Like desktop app: only its runs reach your devices, so this browser's model pickers leave it out.",
				{ name },
			)
		: t(
				"devices:models.use.result.savedNotActiveWeb",
				"“{{name}}” is in your models, but not on in the profile you use. Turn it on in Settings › Models in the Flow-Like desktop app.",
				{ name },
			);
}

function OutcomeLine({
	outcome,
	device,
	desktop,
}: Readonly<{ outcome: Outcome; device: string; desktop: boolean }>) {
	const { t } = useTranslation("devices");
	if (outcome.kind === "failed")
		return (
			<InlineResult tone="critical">
				{failureText(t, outcome.failure, device)}
			</InlineResult>
		);
	return (
		<InlineResult tone={outcome.inProfile ? "good" : "warning"}>
			{savedText(t, outcome.name, outcome.inProfile, desktop)}
		</InlineResult>
	);
}

function consequences(
	t: DevicesT,
	name: string,
	model: string,
	device: string,
	desktop: boolean,
): ConsequenceRows {
	return {
		what: t(
			"devices:models.use.conseq.what",
			"Adds “{{name}}” to your models, served by {{model}} on {{device}}. It holds no secret and copies no weights.",
			{ name, model, device },
		),
		who: t(
			"devices:models.use.conseq.who",
			"You. Whoever runs it needs Use models on {{device}} and unlocks the device on their computer.",
			{ device },
		),
		when: t(
			"devices:models.use.conseq.when",
			"Now. Runs in the desktop app reach the device through an encrypted tunnel. Cloud runs skip it: Find Model takes the next model, and a flow that names it fails there.",
		),
		undo: {
			reversible: true,
			text: desktop
				? t(
						"devices:models.use.conseq.undo",
						"Remove it from your models any time; the model stays on {{device}}.",
						{ device },
					)
				: t(
						"devices:models.use.conseq.undoWeb",
						"Remove it from your models in the Flow-Like desktop app any time; the model stays on {{device}}.",
						{ device },
					),
		},
	};
}

interface FieldsProps {
	formId: string;
	model: UsedModel;
	draft: Draft;
	problems: Problems;
	shown: boolean;
	editable: boolean;
	onChange(patch: Partial<Draft>): void;
}

function UseModelFields({
	formId,
	model,
	draft,
	problems,
	shown,
	editable,
	onChange,
}: Readonly<FieldsProps>) {
	const { t } = useTranslation("devices");
	const tokens = t("devices:models.use.tokens", "tokens");
	const fixed = model.settings.ctx_per_slot !== undefined;
	return (
		<>
			<Field
				id={`${formId}-name`}
				label={t("devices:models.use.name", "Name in your models")}
				error={shown ? problems.name : undefined}
			>
				<DvInput
					value={draft.name}
					maxLength={BIT_NAME_MAX}
					disabled={!editable}
					onChange={(event) => onChange({ name: event.target.value })}
				/>
			</Field>
			<Field
				id={`${formId}-context`}
				label={
					model.kind === "embedding"
						? t("devices:models.use.inputLength", "Input length")
						: t("devices:models.use.context", "Context length")
				}
				hint={
					fixed
						? t(
								"devices:models.use.contextFixed",
								"From the model's settings on the device.",
							)
						: t(
								"devices:models.use.contextOpen",
								"The device picks it; enter what the model was loaded with.",
							)
				}
				error={shown ? problems.context : undefined}
			>
				<InputWithUnit
					numeric
					inputMode="numeric"
					unit={tokens}
					value={draft.context}
					disabled={!editable}
					onChange={(event) => onChange({ context: event.target.value })}
				/>
			</Field>
			{model.kind === "embedding" ? (
				<Field
					id={`${formId}-vector`}
					label={t("devices:models.use.vector", "Vector length")}
					hint={t(
						"devices:models.use.vectorHint",
						"The model card says it, e.g. 768 for nomic-embed-text.",
					)}
					error={shown ? problems.vector : undefined}
				>
					<DvInput
						numeric
						inputMode="numeric"
						value={draft.vector}
						disabled={!editable}
						onChange={(event) => onChange({ vector: event.target.value })}
					/>
				</Field>
			) : null}
		</>
	);
}

interface SheetProps {
	deviceId: string;
	model: UsedModel;
	onClose(): void;
}

/** A failed read only risks a second Bit for the same model, so it doesn't stop the save. */
async function customBits(
	backend: Pick<IBackendState, "bitState">,
): Promise<IBit[]> {
	try {
		return asArray(await backend.bitState.listCustomBits());
	} catch {
		return [];
	}
}

function useSaveDeviceBit(deviceId: string, model: UsedModel) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	return async (draft: Draft, description: string): Promise<Outcome> => {
		const name = draft.name.trim();
		const bit = deviceModelBit({
			id: deviceBitIdFor(await customBits(backend), deviceId, model.id),
			deviceId,
			model,
			name,
			description,
			contextLength: wholeNumber(draft.context, MAX_CONTEXT) ?? 0,
			vectorLength: wholeNumber(draft.vector, MAX_VECTOR_LENGTH),
			now: Date.now(),
		});
		try {
			const saved = await saveDeviceModelBit(backend, bit);
			await Promise.all([
				invalidate(backend.bitState.listCustomBits, []),
				invalidate(backend.bitState.getProfileBits, []),
				invalidate(backend.userState.getSettingsProfile, []),
			]);
			return { kind: "saved", name, inProfile: saved.inProfile };
		} catch (error) {
			return {
				kind: "failed",
				failure: error instanceof DeviceBitError ? error.code : "failed",
			};
		}
	};
}

/** Plan §3.7 "Use from my apps": a Bit for this hosted model in the user's models. */
export function UseModelSheet({
	deviceId,
	model,
	onClose,
}: Readonly<SheetProps>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const { deps } = useDeviceWorkspace();
	const session = useKeySession(deviceId);
	const { setKeepUnlocked } = useKeyChip();
	const save = useSaveDeviceBit(deviceId, model);
	const formId = useId();
	const device = deviceLabel(input, deviceId);
	const desktop = deps.platform === "desktop";
	const [draft, setDraft] = useState<Draft>(() => ({
		name: t("devices:models.use.defaultName", "{{model}} on {{device}}", {
			model: model.display_name,
			device,
		}),
		context: String(defaultContext(model)),
		vector: "",
		keep: true,
	}));
	const [shown, setShown] = useState(false);
	const [saving, setSaving] = useState(false);
	const [outcome, setOutcome] = useState<Outcome | null>(null);
	const unlocked = session.state === "unlocked";
	const problems = problemsOf(t, model, draft);
	const ready = unlocked && Object.keys(problems).length === 0;
	const done = outcome?.kind === "saved";

	const submit = async () => {
		setShown(true);
		if (!ready || saving) return;
		setSaving(true);
		setOutcome(null);
		const result = await save(
			draft,
			t(
				"devices:models.use.description",
				"{{kind}} model hosted on {{device}}.",
				{ kind: kindLabel(t, model.kind), device },
			),
		);
		if (result.kind === "saved" && draft.keep !== session.keepUnlocked)
			setKeepUnlocked(deviceId, draft.keep);
		setOutcome(result);
		setSaving(false);
	};

	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={Plug}
			title={t("devices:models.use.title", "Use {{model}} from your apps", {
				model: model.display_name,
			})}
			sub={
				desktop
					? t(
							"devices:models.use.subtitle",
							"Your flows and chats pick it like any other model, and its calls go to {{device}}.",
							{ device },
						)
					: t(
							"devices:models.use.subtitleWeb",
							"Your flows and chats in the Flow-Like desktop app pick it like any other model, and its calls go to {{device}}.",
							{ device },
						)
			}
			foot={
				done ? (
					<DvButton variant="primary" onClick={onClose}>
						{t("devices:models.use.done", "Done")}
					</DvButton>
				) : (
					<>
						<DvButton onClick={onClose}>
							{t("devices:models.use.cancel", "Cancel")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={Plug}
							busy={saving}
							aria-disabled={!unlocked}
							onClick={() => void submit()}
						>
							{t("devices:models.use.submit", "Add to my models")}
						</DvButton>
					</>
				)
			}
		>
			<UseModelFields
				formId={formId}
				model={model}
				draft={draft}
				problems={problems}
				shown={shown}
				editable={!saving && !done}
				onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))}
			/>
			<KeepForModels
				id={`${formId}-keep`}
				option={{
					checked: draft.keep,
					onChange: (keep) => setDraft((current) => ({ ...current, keep })),
				}}
				disabled={saving || done}
			/>
			{unlocked ? null : (
				<InlineResult tone="warning">
					{t(
						"devices:models.use.locked",
						"{{device}} is locked. Unlock it to add its model.",
						{ device },
					)}
				</InlineResult>
			)}
			<ConsequencePreview
				rows={consequences(
					t,
					draft.name.trim(),
					model.display_name,
					device,
					desktop,
				)}
			/>
			{outcome ? (
				<OutcomeLine outcome={outcome} device={device} desktop={desktop} />
			) : null}
		</DvSheet>
	);
}

/** "Use from my apps…" for one hosted model: needs Use models and an agent that hosts models. */
export function UseModelButton({
	deviceId,
	model,
}: Readonly<{ deviceId: string; model: UsedModel }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const view = gateView(t, time, useGate("models_use", deviceId));
	const [open, setOpen] = useState(false);
	return (
		<>
			<GatedAction gate={view?.gate}>
				<DvButton
					size="xs"
					variant="ghost"
					icon={Plug}
					onClick={() => setOpen(true)}
				>
					{t("devices:models.use.button", "Use from my apps…")}
				</DvButton>
			</GatedAction>
			{open ? (
				<UseModelSheet
					deviceId={deviceId}
					model={model}
					onClose={() => setOpen(false)}
				/>
			) : null}
		</>
	);
}
