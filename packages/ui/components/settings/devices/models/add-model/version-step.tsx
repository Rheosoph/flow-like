"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import {
	type HostFacts,
	type ModelFit,
	gpuLeftOut,
} from "../../../../../lib/device-management/model/models/fit";
import type {
	ModelEngine,
	ModelKind,
} from "../../../../../lib/device-management/models";
import { bytesText } from "../../observe/observe-data";
import type { DevicesT } from "../../primitives/area-context";
import {
	ChoiceCards,
	type ChoiceOption,
	DvSelect,
	type DvSelectOption,
	Field,
} from "../../primitives/form-fields";
import { GateInline } from "../../primitives/gate-notice";
import { type SegmentOption, Segmented } from "../../primitives/segmented";
import { StatusChip } from "../../primitives/status-chip";
import { engineLabel, kindLabel } from "../models-copy";
import {
	VERDICT_TONE,
	blockText,
	estimatedNote,
	fitDetail,
	gpuLeftOutNote,
	memoryText,
	speedText,
	verdictLabel,
} from "./fit-copy";
import type { OptionView } from "./install-plan";
import type { ModelCandidate } from "./model-options";

export interface VersionStepProps {
	candidate: ModelCandidate;
	views: readonly OptionView[];
	selected: OptionView;
	onSelect(optionId: string): void;
	kind: ModelKind;
	onKind(kind: ModelKind): void;
	projector?: string;
	onProjector(fileName: string): void;
	device: string;
	host: HostFacts;
}

function kindOptions(t: DevicesT, kinds: readonly ModelKind[]) {
	const options: SegmentOption<ModelKind>[] = [];
	for (const kind of kinds)
		options.push({ value: kind, label: kindLabel(t, kind) });
	return options;
}

function FitChips({
	view,
	many,
}: Readonly<{ view: OptionView; many: boolean }>) {
	const { t } = useTranslation("devices");
	const fit = view.advice.fit;
	return (
		<span className="flex flex-wrap items-center gap-1.5">
			{view.option.recommended && many ? (
				<StatusChip tone="info">
					{t("devices:models.install.version.recommended", "Recommended")}
				</StatusChip>
			) : null}
			{fit ? (
				<StatusChip tone={VERDICT_TONE[fit.verdict]}>
					{verdictLabel(t, fit.verdict)}
				</StatusChip>
			) : null}
		</span>
	);
}

/** "4.7 GiB download · About 10.3 GiB of GPU memory · Fast · about 83 tokens/s". */
function versionHint(t: DevicesT, view: OptionView) {
	const fit = view.advice.fit;
	const parts = [
		view.download.files
			? t("devices:models.install.version.download", "{{size}} download", {
					size: bytesText(view.download.bytes),
				})
			: t("devices:models.install.version.present", "Already on the device"),
		fit ? memoryText(t, fit) : "",
		fit ? (speedText(t, fit) ?? "") : "",
	];
	return parts.filter(Boolean).join(" · ");
}

function FitNotes({
	fit,
	host,
	engine,
}: Readonly<{ fit: ModelFit; host: HostFacts; engine: ModelEngine }>) {
	const { t } = useTranslation("devices");
	const leftOut = gpuLeftOut(host, engine);
	return (
		<span className="flex flex-col gap-1 text-xs text-muted-foreground">
			<span className="text-ink-2">{fitDetail(t, fit)}</span>
			{fit.estimatedShape ? <span>{estimatedNote(t)}</span> : null}
			{leftOut ? <span>{gpuLeftOutNote(t, leftOut)}</span> : null}
		</span>
	);
}

function VersionDetail({
	view,
	props,
}: Readonly<{ view: OptionView; props: Readonly<VersionStepProps> }>) {
	const { t } = useTranslation("devices");
	const fit = view.advice.fit;
	return (
		<span data-version-detail="" className="flex flex-col gap-1.5">
			{fit ? (
				<FitNotes fit={fit} host={props.host} engine={view.option.engine} />
			) : null}
			{view.block ? (
				<GateInline kind="unsupported" className="max-w-none">
					{blockText(t, view.block, {
						device: props.device,
						engine: engineLabel(t, view.option.engine),
						needed: view.disk.needed,
						available: view.disk.available,
					})}
				</GateInline>
			) : null}
		</span>
	);
}

function versionOptions(
	t: DevicesT,
	props: Readonly<VersionStepProps>,
): ChoiceOption<string>[] {
	const options: ChoiceOption<string>[] = [];
	for (const view of props.views)
		options.push({
			value: view.option.id,
			title: (
				<span className="flex flex-wrap items-center gap-x-2 gap-y-1">
					<span>{view.option.label || props.candidate.name}</span>
					<FitChips view={view} many={props.views.length > 1} />
				</span>
			),
			hint: versionHint(t, view),
			detail: <VersionDetail view={view} props={props} />,
		});
	return options;
}

function projectorOptions(view: OptionView) {
	const options: DvSelectOption<string>[] = [];
	for (const file of view.option.projectors)
		options.push({
			value: file.file_name,
			label: `${file.file_name} · ${bytesText(file.size)}`,
		});
	return options;
}

function ProjectorField(props: Readonly<VersionStepProps>) {
	const { t } = useTranslation("devices");
	const { selected } = props;
	const needed =
		(props.kind === "vision" ||
			(props.kind === "systemone" && selected.option.projectors.length > 0)) &&
		selected.option.engine === "llamacpp" &&
		selected.option.projectors.length > 1;
	if (!needed) return null;
	return (
		<Field
			id="add-model-projector"
			label={t("devices:models.install.version.projector", "Projector")}
			hint={t(
				"devices:models.install.version.projectorHint",
				"The file that lets the model see images. The first one is recommended.",
			)}
		>
			<DvSelect
				value={props.projector ?? selected.option.projectors[0]?.file_name}
				onValueChange={props.onProjector}
				options={projectorOptions(selected)}
				mono
			/>
		</Field>
	);
}

/** Step 2: the version (a quantization) with how it fits on the device. */
export function VersionStep(props: Readonly<VersionStepProps>) {
	const { t } = useTranslation("devices");
	const { candidate } = props;
	let kinds: ReactNode = null;
	if (candidate.kinds.length > 1)
		kinds = (
			<Segmented
				label={t("devices:models.install.version.kind", "Use it for")}
				options={kindOptions(t, candidate.kinds)}
				value={props.kind}
				onChange={props.onKind}
			/>
		);
	return (
		<div data-step="version" className="flex flex-col gap-3.5">
			<p className="text-sm">
				<b className="font-semibold">{candidate.name}</b>
				{candidate.license ? (
					<span className="text-muted-foreground">
						{" · "}
						{t(
							"devices:models.install.version.license",
							"License: {{license}}",
							{
								license: candidate.license,
							},
						)}
					</span>
				) : null}
			</p>
			{kinds}
			<ChoiceCards
				id="add-model-version"
				legend={t(
					"devices:models.install.version.legend",
					"Version, and how it fits on {{device}}",
					{ device: props.device },
				)}
				value={props.selected.option.id}
				onValueChange={props.onSelect}
				options={versionOptions(t, props)}
			/>
			<ProjectorField {...props} />
		</div>
	);
}
