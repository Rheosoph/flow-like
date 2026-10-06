"use client";

import { useTranslation } from "@flow-like/locales";
import { Boxes, Plus } from "lucide-react";
import { type ReactNode, useEffect, useId, useMemo, useState } from "react";
import {
	engineSettings,
	fitModel,
} from "../../../../../lib/device-management/model/models/fit";
import type { AgentFeatures } from "../../../../../lib/device-management/model/types";
import type {
	ModelKind,
	ModelSettings,
	Residency,
} from "../../../../../lib/device-management/models";
import { gateView } from "../../device/use-device-page";
import {
	type ObserveTarget,
	useObserveTarget,
} from "../../observe/use-observe-target";
import type { OverlaySheetProps } from "../../overlays/area-overlays";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { GateInline } from "../../primitives/gate-notice";
import type { GateKind } from "../../primitives/icons";
import { useGate } from "../../workspace";
import { adviceView } from "../settings/hosted-advice";
import { SettingsForm } from "../settings/settings-form";
import { OverviewPending } from "../settings/sheet-states";
import {
	useModelsAccess,
	useModelsAfter,
	useModelsOverview,
	withAllModels,
} from "../use-models";
import { memoryText, verdictLabel } from "./fit-copy";
import {
	type DeviceModelFacts,
	type OptionView,
	deviceFactsOf,
	hostedTwin,
	inlineInstallFits,
	optionViews,
} from "./install-plan";
import {
	type ModelCandidate,
	choiceFacts,
	displayNameOf,
	modelIdFor,
} from "./model-options";
import type { Fetcher } from "./model-reads";
import { DoneView, ReviewStep } from "./review-step";
import { SourceStep } from "./source-step";
import {
	type SourcePick,
	useCandidateLookup,
	useInstall,
	useSourceLists,
} from "./use-add-model";
import { VersionStep } from "./version-step";

export interface AddModelSheetProps extends OverlaySheetProps {
	deviceId: string;
	/** Reads Hugging Face and small files; `fetch` by default. */
	fetcher?: Fetcher;
}

const STEPS = ["source", "version", "settings", "review"] as const;
type Step = (typeof STEPS)[number];

interface Selection {
	optionId: string;
	kind: ModelKind;
	projector?: string;
}

interface Draft {
	/** The choice the draft was made for: another choice starts from its own advice. */
	key: string;
	settings: ModelSettings;
	residency: Residency;
}

const defaultFetch: Fetcher = (input, init) => fetch(input, init);

function stepTitle(t: DevicesT, step: Step) {
	const titles: Record<Step, string> = {
		source: t("devices:models.install.step.source", "Model"),
		version: t("devices:models.install.step.version", "Version"),
		settings: t("devices:models.install.step.settings", "Settings"),
		review: t("devices:models.install.step.review", "Review"),
	};
	return titles[step];
}

const choiceKey = (view: OptionView) =>
	[
		view.choice.candidate.key,
		view.option.id,
		view.choice.kind,
		view.choice.projector ?? "",
	].join("|");

function versionsOf(
	candidate: ModelCandidate | undefined,
	selection: Selection | undefined,
	device: DeviceModelFacts,
) {
	if (!candidate || !selection) return { views: [], selected: undefined };
	const views = optionViews(
		candidate,
		{ kind: selection.kind, projector: selection.projector },
		device,
	);
	const selected =
		views.find((view) => view.option.id === selection.optionId) ?? views[0];
	return { views, selected };
}

interface WizardProps {
	target: ObserveTarget;
	device: DeviceModelFacts;
	fetcher: Fetcher;
	onClose(): void;
}

function useWizard({ target, device, fetcher }: Readonly<WizardProps>) {
	const [step, setStep] = useState<Step>("source");
	const [pick, setPick] = useState<SourcePick>({
		source: "hub",
		reference: "",
	});
	const [candidate, setCandidate] = useState<ModelCandidate>();
	const [selection, setSelection] = useState<Selection>();
	const [draft, setDraft] = useState<Draft>();
	const lists = useSourceLists(pick.source);
	const lookup = useCandidateLookup(fetcher);
	const installer = useInstall(target.deviceId, fetcher);
	const { views, selected } = useMemo(
		() => versionsOf(candidate, selection, device),
		[candidate, selection, device],
	);
	const lookUp = async () => {
		const found = await lookup.find(pick);
		if (!found) return;
		setCandidate(found);
		setSelection({
			optionId: found.options[0]?.id ?? "",
			kind: found.kinds[0] ?? "chat",
		});
		setStep("version");
	};
	const toSettings = (view: OptionView) => {
		if (draft?.key !== choiceKey(view))
			setDraft({
				key: choiceKey(view),
				settings: view.advice.settings,
				residency: view.advice.residency,
			});
		setStep("settings");
	};
	return {
		step,
		setStep,
		pick,
		setPick: (next: SourcePick) => {
			lookup.clear();
			setPick(next);
		},
		lists,
		lookup,
		lookUp,
		candidate,
		selection,
		setSelection,
		views,
		selected,
		draft,
		setDraft,
		toSettings,
		installer,
	};
}

type Wizard = ReturnType<typeof useWizard>;

interface Reason {
	kind: GateKind;
	text: string;
}

function sourceReason(t: DevicesT, wizard: Wizard): Reason | undefined {
	const { pick } = wizard;
	const ready =
		pick.source === "huggingface" ? pick.reference.trim() !== "" : !!pick.bit;
	if (ready) return undefined;
	return {
		kind: "busy",
		text:
			pick.source === "huggingface"
				? t(
						"devices:models.install.reason.reference",
						"Enter a Hugging Face repository first.",
					)
				: t("devices:models.install.reason.pick", "Choose a model first."),
	};
}

/** Why the primary is off at this step; undefined when it's on. */
function stepReason(
	t: DevicesT,
	wizard: Wizard,
	gate: Reason | undefined,
	installFits: boolean,
): Reason | undefined {
	if (wizard.step === "source") return sourceReason(t, wizard);
	if (wizard.step === "version" && wizard.selected?.block)
		return {
			kind: "unsupported",
			text: t(
				"devices:models.install.reason.blocked",
				"This version can't be added to the device.",
			),
		};
	if (wizard.step !== "review") return undefined;
	return (
		gate ??
		(installFits
			? undefined
			: {
					kind: "unsupported",
					text: t(
						"devices:models.install.reason.requestTooLarge",
						"This model's file list exceeds the 16 KiB limit for adding it here. Choose a version with fewer files or include the model in an app deployment.",
					),
				})
	);
}

function SettingsStep({
	wizard,
	view,
	device,
}: Readonly<{ wizard: Wizard; view: OptionView; device: DeviceModelFacts }>) {
	const { t } = useTranslation("devices");
	const draft = wizard.draft as Draft;
	const facts = choiceFacts(view.choice);
	const cores = device.system.cpu.physical_cores;
	const fit = fitModel(facts, draft.settings, device);
	return (
		<div data-step="settings" className="flex flex-col gap-3.5">
			<output data-fit={fit.verdict} className="block text-ui">
				<b className="font-semibold">{verdictLabel(t, fit.verdict)}</b>
				{" · "}
				{memoryText(t, fit)}
			</output>
			<SettingsForm
				id="add-model-settings"
				engine={view.option.engine}
				value={draft.settings}
				onChange={(settings) => wizard.setDraft({ ...draft, settings })}
				residency={draft.residency}
				onResidency={(residency) => wizard.setDraft({ ...draft, residency })}
				advice={adviceView(t, view.advice, {
					contextLength: facts.contextLength,
					cores,
				})}
				contextLimit={facts.contextLength}
				cores={cores}
			/>
		</div>
	);
}

function TwinNote({
	view,
	device,
	name,
}: Readonly<{ view: OptionView; device: DeviceModelFacts; name: string }>) {
	const { t } = useTranslation("devices");
	const twin = hostedTwin(view.choice, device);
	if (!twin) return null;
	return (
		<Banner tone="warning">
			{t(
				"devices:models.install.review.twin",
				"{{device}} already hosts these files as {{model}}. Adding them again creates a second model with its own settings; the files aren't downloaded twice.",
				{ device: name, model: twin.display_name },
			)}
		</Banner>
	);
}

function StepBody({
	wizard,
	device,
	target,
}: Readonly<{
	wizard: Wizard;
	device: DeviceModelFacts;
	target: ObserveTarget;
}>) {
	const { step, selected, candidate, selection } = wizard;
	if (step === "source" || !candidate || !selected || !selection)
		return (
			<SourceStep
				{...wizard.lists}
				pick={wizard.pick}
				onPick={wizard.setPick}
				error={wizard.lookup.error}
				onSubmit={() => void wizard.lookUp()}
			/>
		);
	if (step === "version")
		return (
			<VersionStep
				candidate={candidate}
				views={wizard.views}
				selected={selected}
				onSelect={(optionId) => wizard.setSelection({ ...selection, optionId })}
				kind={selection.kind}
				onKind={(kind) => wizard.setSelection({ ...selection, kind })}
				projector={selection.projector}
				onProjector={(projector) =>
					wizard.setSelection({ ...selection, projector })
				}
				device={target.name}
				host={device}
			/>
		);
	if (step === "settings")
		return <SettingsStep wizard={wizard} view={selected} device={device} />;
	return (
		<ReviewBody
			wizard={wizard}
			view={selected}
			device={device}
			target={target}
		/>
	);
}

function ReviewBody({
	wizard,
	view,
	device,
	target,
}: Readonly<{
	wizard: Wizard;
	view: OptionView;
	device: DeviceModelFacts;
	target: ObserveTarget;
}>) {
	const draft = wizard.draft as Draft;
	const name = displayNameOf(view.choice);
	return (
		<>
			<TwinNote view={view} device={device} name={target.name} />
			<ReviewStep
				deviceId={target.deviceId}
				device={target.name}
				view={view}
				name={name}
				settings={draft.settings}
				residency={draft.residency}
				host={device}
				phase={wizard.installer.phase}
			/>
		</>
	);
}

function primaryLabel(t: DevicesT, wizard: Wizard) {
	if (wizard.step !== "review" || !wizard.selected)
		return t("devices:models.install.continue", "Continue");
	return t("devices:models.install.add", "Add {{model}}", {
		model: displayNameOf(wizard.selected.choice),
	});
}

function installInputOf(wizard: Wizard, device: DeviceModelFacts) {
	const { selected, draft } = wizard;
	if (!selected || !draft) return undefined;
	return {
		choice: selected.choice,
		modelId: modelIdFor(
			displayNameOf(selected.choice),
			new Set(device.models.map((model) => model.id)),
		),
		settings: engineSettings(selected.option.engine, draft.settings),
		residency: draft.residency,
	};
}

function advanceOf(wizard: Wizard, device: DeviceModelFacts) {
	return () => {
		const { step, selected } = wizard;
		if (step === "source") return void wizard.lookUp();
		if (!selected) return;
		if (step === "version") return wizard.toSettings(selected);
		if (step === "settings") return wizard.setStep("review");
		const input = installInputOf(wizard, device);
		if (input) void wizard.installer.install(input);
	};
}

const busyPhase = (wizard: Wizard) =>
	wizard.installer.phase.kind === "fingerprinting" ||
	wizard.installer.phase.kind === "sending";

function WizardFoot({
	wizard,
	reason,
	reasonId,
	onAdvance,
	onClose,
}: Readonly<{
	wizard: Wizard;
	reason: Reason | undefined;
	reasonId: string;
	onAdvance(): void;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const index = STEPS.indexOf(wizard.step);
	const busy = wizard.lookup.busy || busyPhase(wizard);
	return (
		<>
			{index > 0 ? (
				<DvButton
					disabled={busy}
					onClick={() => wizard.setStep(STEPS[index - 1] as Step)}
				>
					{t("devices:models.install.back", "Back")}
				</DvButton>
			) : (
				<DvButton onClick={onClose}>
					{t("devices:models.install.cancel", "Cancel")}
				</DvButton>
			)}
			<DvButton
				variant="primary"
				icon={wizard.step === "review" ? Plus : undefined}
				busy={busy}
				aria-disabled={reason ? true : undefined}
				aria-describedby={reason ? reasonId : undefined}
				data-act="add-model-next"
				onClick={() => {
					if (!reason) onAdvance();
				}}
			>
				{primaryLabel(t, wizard)}
			</DvButton>
		</>
	);
}

function WizardSheet(props: Readonly<WizardProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const reasonId = useId();
	const wizard = useWizard(props);
	const advance = advanceOf(wizard, props.device);
	const manage = useGate("models_manage", props.target.deviceId);
	const gateFailure = gateView(t, time, manage)?.gate;
	const gate = gateFailure
		? { kind: gateFailure.kind, text: String(gateFailure.reason) }
		: undefined;
	const installInput = installInputOf(wizard, props.device);
	const installFits =
		!installInput || inlineInstallFits(installInput, props.target.deviceId);
	const reason = stepReason(t, wizard, gate, installFits);
	const index = STEPS.indexOf(wizard.step);
	const phase = wizard.installer.phase;
	const done = phase.kind === "done";
	const settled = !done && !wizard.lookup.busy && !busyPhase(wizard);
	const note: ReactNode = reason ? (
		<GateInline kind={reason.kind} id={reasonId}>
			{reason.text}
		</GateInline>
	) : (
		t(
			"devices:models.install.stepNote",
			"Step {{n, number}} of {{total, number}} · {{step}}",
			{
				n: index + 1,
				total: STEPS.length,
				step: stepTitle(t, wizard.step),
			},
		)
	);
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) props.onClose();
			}}
			wide
			icon={Boxes}
			title={t("devices:models.install.title", "Add a model to {{device}}", {
				device: props.target.name,
			})}
			sub={done ? undefined : stepTitle(t, wizard.step)}
			onBack={
				index > 0 && settled
					? () => wizard.setStep(STEPS[index - 1] as Step)
					: undefined
			}
			footNote={done ? undefined : note}
			foot={
				done ? (
					<DvButton variant="primary" onClick={props.onClose}>
						{t("devices:models.install.close", "Close")}
					</DvButton>
				) : (
					<WizardFoot
						wizard={wizard}
						reason={reason}
						reasonId={reasonId}
						onAdvance={advance}
						onClose={props.onClose}
					/>
				)
			}
		>
			{done ? (
				<DoneView result={phase.result} device={props.target.name} />
			) : (
				<StepBody wizard={wizard} device={props.device} target={props.target} />
			)}
		</DvSheet>
	);
}

/**
 * The device's facts with every hosted model: the overview lists only those
 * that fit one reply, so the rest are read page by page and their ids, files
 * and twins count too. Once complete, the facts stay while a later read pages
 * again (the overview's cut moves as models change); locking drops them.
 */
function useDeviceFacts(deviceId: string, features: AgentFeatures | undefined) {
	const overview = useModelsOverview(deviceId, { poll: false });
	const listed = overview.data;
	const after = useModelsAfter(deviceId, listed?.next ?? null, {
		poll: false,
	});
	const paging = !!listed?.next && after.data === undefined;
	const fresh = useMemo(
		() =>
			listed && !paging
				? deviceFactsOf(withAllModels(listed, after.data), features)
				: undefined,
		[listed, paging, after.data, features],
	);
	const [complete, setComplete] = useState<DeviceModelFacts>();
	useEffect(() => {
		if (!listed) setComplete(undefined);
		else if (fresh) setComplete(fresh);
	}, [listed, fresh]);
	return {
		device: listed ? (fresh ?? complete) : undefined,
		overview: after.failure
			? { ...overview, failure: after.failure, refetch: after.refetch }
			: overview,
	};
}

/**
 * Models tab › Add model (plan §3.7): a model from the Flow-Like hub, a
 * Hugging Face repository or the person's own Bits; a version with its fit on
 * this device; settings with recommended values; the consequences; one
 * `install` whose every file carries a pinned digest.
 */
export function AddModelSheet({
	deviceId,
	scope,
	onClose,
	fetcher = defaultFetch,
}: Readonly<AddModelSheetProps>) {
	const { t } = useTranslation("devices");
	const target = useObserveTarget(deviceId, null, scope);
	const access = useModelsAccess(deviceId);
	const { device, overview } = useDeviceFacts(deviceId, access.features);
	if (device)
		return (
			<WizardSheet
				target={target}
				device={device}
				fetcher={fetcher}
				onClose={onClose}
			/>
		);
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={Boxes}
			title={t("devices:models.install.title", "Add a model to {{device}}", {
				device: target.name,
			})}
			foot={
				<DvButton onClick={onClose}>
					{t("devices:models.install.close", "Close")}
				</DvButton>
			}
		>
			<OverviewPending target={target} access={access} overview={overview} />
		</DvSheet>
	);
}
