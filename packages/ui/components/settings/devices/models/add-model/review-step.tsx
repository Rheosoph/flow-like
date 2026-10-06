"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleCheck } from "lucide-react";
import {
	type HostFacts,
	type ModelFit,
	fitModel,
} from "../../../../../lib/device-management/model/models/fit";
import type {
	ModelInstalled,
	ModelSettings,
	Residency,
} from "../../../../../lib/device-management/models";
import { bytesText } from "../../observe/observe-data";
import type { DevicesT } from "../../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../../primitives/consequence-preview";
import { InlineResult } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { useInlineResults } from "../../workspace";
import { installCopy } from "../action-copy";
import { engineLabel, kindLabel, residencyLabel } from "../models-copy";
import { modelsResultKey } from "../use-models-action";
import { memoryText, verdictLabel } from "./fit-copy";
import type { OptionView } from "./install-plan";
import { choiceBytes, choiceFacts, choiceFiles } from "./model-options";
import type { InstallPhase } from "./use-add-model";

export interface ReviewStepProps {
	deviceId: string;
	device: string;
	view: OptionView;
	name: string;
	settings: ModelSettings;
	residency: Residency;
	host: HostFacts;
	phase: InstallPhase;
}

const hostOf = (source: string) => {
	try {
		return new URL(source).host;
	} catch {
		return undefined;
	}
};

function sourceHosts(view: OptionView) {
	const hosts = new Set<string>();
	for (const file of choiceFiles(view.choice)) {
		const host = file.sources[0] ? hostOf(file.sources[0]) : undefined;
		if (host) hosts.add(host);
	}
	return [...hosts].join(", ");
}

function diskAndMemory(t: DevicesT, view: OptionView, fit: ModelFit) {
	return t(
		"devices:models.install.review.diskMemory",
		"Takes {{disk}} of the model disk, which has {{left}} left. Files another model already uses aren't downloaded again. Once loaded: {{memory}} ({{verdict}}).",
		{
			disk: bytesText(view.download.bytes),
			left: bytesText(view.disk.available),
			memory: memoryText(t, fit),
			verdict: verdictLabel(t, fit.verdict).toLowerCase(),
		},
	);
}

/** What the install does: the download, the disk and memory it takes, how to undo it. */
function installRows(
	t: DevicesT,
	props: Readonly<ReviewStepProps>,
	fit: ModelFit,
): ConsequenceRows {
	const { view, device, name } = props;
	const rows = installCopy(t, {
		model: name,
		device,
		size: view.download.bytes,
		files: view.download.files,
	}).rows;
	const hosts = sourceHosts(view);
	const sources = hosts
		? ` ${t("devices:models.install.review.sources", "Sources: {{hosts}}.", {
				hosts,
			})}`
		: "";
	const what = view.download.files
		? `${String(rows.what)}${sources}`
		: t(
				"devices:models.install.review.whatPresent",
				"{{device}} has every file of it already, so nothing is downloaded.",
				{ device },
			);
	return { ...rows, what, stays: diskAndMemory(t, view, fit) };
}

function Summary(props: Readonly<ReviewStepProps>) {
	const { t } = useTranslation("devices");
	const { view } = props;
	const files = choiceFiles(view.choice).length;
	return (
		<KeyValueList>
			<KvRow label={t("devices:models.install.review.model", "Model")}>
				{props.name}
			</KvRow>
			<KvRow label={t("devices:models.install.review.kind", "Used for")}>
				{`${kindLabel(t, view.choice.kind)} · ${engineLabel(t, view.option.engine)}`}
			</KvRow>
			<KvRow label={t("devices:models.install.review.files", "Files")}>
				{t("devices:models.install.review.filesValue", {
					count: files,
					size: bytesText(choiceBytes(view.choice)),
					defaultValue_one: "{{count, number}} file · {{size}}",
					defaultValue_other: "{{count, number}} files · {{size}}",
				})}
			</KvRow>
			<KvRow label={t("devices:models.install.review.runs", "When it runs")}>
				{residencyLabel(t, props.residency)}
			</KvRow>
		</KeyValueList>
	);
}

function PhaseLine({ phase }: Readonly<{ phase: InstallPhase }>) {
	const { t } = useTranslation("devices");
	if (phase.kind === "fingerprinting" && phase.total > 0)
		return (
			<InlineResult tone="info">
				{t("devices:models.install.review.fingerprinting", {
					count: phase.total,
					done: phase.done,
					defaultValue_one:
						"Fingerprinting {{done, number}} of {{count, number}} small file on this computer…",
					defaultValue_other:
						"Fingerprinting {{done, number}} of {{count, number}} small files on this computer…",
				})}
			</InlineResult>
		);
	if (phase.kind === "sending")
		return (
			<InlineResult tone="info">
				{t(
					"devices:models.install.review.sending",
					"Sending it to the device…",
				)}
			</InlineResult>
		);
	if (phase.kind === "failed")
		return (
			<InlineResult tone="critical">
				{t(
					"devices:models.install.review.fingerprintFailed",
					"Nothing was sent: {{reason}}",
					{ reason: phase.message },
				)}
			</InlineResult>
		);
	return null;
}

/** Step 4: what happens, then Add. */
export function ReviewStep(props: Readonly<ReviewStepProps>) {
	const { t } = useTranslation("devices");
	const results = useInlineResults(modelsResultKey.install(props.deviceId));
	const fit = fitModel(
		choiceFacts(props.view.choice),
		props.settings,
		props.host,
	);
	return (
		<div data-step="review" className="flex flex-col gap-3.5">
			<Summary {...props} />
			<ConsequencePreview
				rows={installRows(t, props, fit)}
				labels={{
					stays: t(
						"devices:models.install.review.diskMemoryLabel",
						"Disk and memory",
					),
				}}
			/>
			<PhaseLine phase={props.phase} />
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}

function doneText(t: DevicesT, left: number) {
	if (!left)
		return t(
			"devices:models.install.done.present",
			"The device has every file already. It loads when you load it or on its first request.",
		);
	return t("devices:models.install.done.downloading", {
		count: left,
		defaultValue_one:
			"The device downloads {{count, number}} file and checks it. Progress shows under Downloads on the Models tab.",
		defaultValue_other:
			"The device downloads {{count, number}} files and checks each one. Progress shows under Downloads on the Models tab.",
	});
}

/** After the install: the device took it and downloads in the background. */
export function DoneView({
	result,
	device,
}: Readonly<{ result: ModelInstalled; device: string }>) {
	const { t } = useTranslation("devices");
	return (
		<output
			data-done=""
			className="flex items-start gap-3 rounded-lg border border-good-line bg-good-bg p-4"
		>
			<CircleCheck aria-hidden className="mt-0.5 size-4 shrink-0 text-good" />
			<span className="flex min-w-0 flex-col gap-1">
				<b className="text-sm font-semibold">
					{t(
						"devices:models.install.done.title",
						"{{model}} is on {{device}}",
						{
							model: result.model.display_name,
							device,
						},
					)}
				</b>
				<span className="text-ui text-ink-2">
					{doneText(t, result.assets.total - result.assets.present)}
				</span>
			</span>
		</output>
	);
}
