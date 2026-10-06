"use client";

import { useTranslation } from "@flow-like/locales";
import { Diff, RefreshCw, Settings2 } from "lucide-react";
import { useMemo, useState } from "react";
import { flashForCache } from "../../../../../lib/device-management/model/models/fit";
import type {
	HostedModel,
	ModelSettings,
	ModelsOverview,
	Residency,
} from "../../../../../lib/device-management/models";
import { gateView } from "../../device/use-device-page";
import { bytesText } from "../../observe/observe-data";
import {
	type ObserveTarget,
	useObserveTarget,
} from "../../observe/use-observe-target";
import type { OverlaySheetProps } from "../../overlays/area-overlays";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DiffRows } from "../../primitives/diff-rows";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { GatedAction } from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import { StateView } from "../../primitives/state-view";
import { useInlineResults } from "../../workspace";
import { engineLabel, kindLabel } from "../models-copy";
import {
	type ModelsRead,
	useModelsAccess,
	useModelsAfter,
	useModelsOverview,
	withAllModels,
} from "../use-models";
import { useModelsAction } from "../use-models-action";
import {
	configuredSettings,
	editsSince,
	hostedAdvice,
	rebasedDraft,
	settingsChanges,
} from "./hosted-advice";
import { SettingsForm } from "./settings-form";
import { OverviewPending } from "./sheet-states";

export interface ModelSettingsSheetProps extends OverlaySheetProps {
	deviceId: string;
	modelId: string;
}

interface EditorProps {
	target: ObserveTarget;
	model: HostedModel;
	overview: ModelsRead<ModelsOverview> & { data: ModelsOverview };
	onClose(): void;
}

function memoryNow(t: DevicesT, model: HostedModel, kind: string) {
	if (model.state !== "loaded") return kind;
	const ram = bytesText(model.ram_bytes);
	if (!model.vram_bytes)
		return t(
			"devices:models.settings.memoryNowRam",
			"{{kind}} · uses {{ram}} of RAM now",
			{ kind, ram },
		);
	return t(
		"devices:models.settings.memoryNow",
		"{{kind}} · uses {{gpu}} of GPU memory and {{ram}} of RAM now",
		{ kind, gpu: bytesText(model.vram_bytes), ram },
	);
}

/** "Chat · llama.cpp · uses 6.4 GiB of GPU memory and 922 MiB of RAM now". */
function ModelFacts({ model }: Readonly<{ model: HostedModel }>) {
	const { t } = useTranslation("devices");
	const kind = `${kindLabel(t, model.kind)} · ${engineLabel(t, model.engine)}`;
	return (
		<p className="text-xs text-muted-foreground">{memoryNow(t, model, kind)}</p>
	);
}

function StaleBanner({ onReload }: Readonly<{ onReload(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<Banner
			tone="warning"
			title={t(
				"devices:models.settings.stale.title",
				"These settings changed on the device",
			)}
			actions={
				<DvButton size="sm" icon={RefreshCw} onClick={onReload}>
					{t("devices:models.settings.stale.reload", "Load the new settings")}
				</DvButton>
			}
		>
			{t(
				"devices:models.settings.stale.text",
				"Someone changed them after you opened this sheet. Load them, then apply your change again; what you changed here stays.",
			)}
		</Banner>
	);
}

/** Where the sheet's base revision comes from, and a reload that rebases once the read is back. */
function useBase(
	model: HostedModel,
	overview: EditorProps["overview"],
	onRebase: (from: HostedModel, to: HostedModel) => void,
) {
	const [base, setBase] = useState(model);
	const [conflict, setConflict] = useState(false);
	const [rebase, setRebase] = useState(false);
	if (rebase) {
		setRebase(false);
		onRebase(base, model);
		setBase(model);
		setConflict(false);
	}
	return {
		base,
		stale: conflict || model.revision !== base.revision,
		conflict: () => setConflict(true),
		reload: async () => {
			await overview.refetch();
			setRebase(true);
		},
	};
}

function useEditor({
	target,
	model,
	overview,
	onClose,
}: Readonly<EditorProps>) {
	const { t } = useTranslation("devices");
	const [draft, setDraft] = useState<ModelSettings>(() =>
		flashForCache(model.engine, model.settings),
	);
	const [residency, setResidency] = useState<Residency>(model.residency);
	const [reviewing, setReviewing] = useState(false);
	const { base, stale, conflict, reload } = useBase(
		model,
		overview,
		(from, to) => {
			const next = rebasedDraft(editsSince(from, draft, residency), to);
			setDraft(next.settings);
			setResidency(next.residency);
		},
	);
	const advice = useMemo(
		() => hostedAdvice(t, base, overview.data, { device: target.name }),
		[t, base, overview.data, target.name],
	);
	const change = { settings: configuredSettings(base, draft), residency };
	const command = useModelsAction(target.deviceId).configure(base, change);
	const apply = async () => {
		const outcome = await command.run({ confirmed: true });
		if (outcome.status === "done") onClose();
		if (
			outcome.status === "rejected" &&
			outcome.rejection.code === "revision_conflict"
		)
			conflict();
	};
	return {
		base,
		draft,
		setDraft,
		residency,
		setResidency,
		reviewing,
		setReviewing,
		stale,
		reload,
		advice,
		command,
		apply,
		cores: overview.data.system.cpu.physical_cores,
		changes: settingsChanges(t, base, change),
	};
}

type Editor = ReturnType<typeof useEditor>;

function ReviewBody({ editor }: Readonly<{ editor: Editor }>) {
	const { t } = useTranslation("devices");
	const results = useInlineResults(editor.command.resultKey);
	return (
		<>
			<DiffRows
				rows={editor.changes}
				label={t("devices:models.settings.changes", "Changes")}
				className="overflow-hidden rounded-lg border border-border bg-card"
			/>
			<ConsequencePreview rows={editor.command.rows} />
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</>
	);
}

function ChangeBody({ editor }: Readonly<{ editor: Editor }>) {
	const { base } = editor;
	return (
		<>
			<ModelFacts model={base} />
			<SettingsForm
				id={`model-settings-${base.id}`}
				engine={base.engine}
				value={editor.draft}
				onChange={editor.setDraft}
				residency={editor.residency}
				onResidency={editor.setResidency}
				advice={editor.advice}
				cores={editor.cores}
				deviceDefaults
			/>
		</>
	);
}

function EditorFoot({ editor }: Readonly<{ editor: Editor }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const gate = gateView(t, time, editor.command.gate)?.gate;
	if (!editor.reviewing)
		return (
			<>
				<span className="text-xs text-muted-foreground">
					{editor.changes.length
						? null
						: t("devices:models.settings.noChanges", "Nothing changed yet.")}
				</span>
				<DvButton
					variant="primary"
					aria-disabled={editor.changes.length ? undefined : true}
					data-act="settings-review"
					onClick={() => {
						if (editor.changes.length) editor.setReviewing(true);
					}}
				>
					{t("devices:models.settings.review", "Review changes")}
				</DvButton>
			</>
		);
	return (
		<>
			<DvButton onClick={() => editor.setReviewing(false)}>
				{t("devices:models.settings.back", "Back")}
			</DvButton>
			<GatedAction gate={gate}>
				<DvButton
					variant="primary"
					busy={editor.command.pending}
					aria-disabled={editor.stale || undefined}
					data-act="settings-apply"
					onClick={() => {
						if (!editor.stale) void editor.apply();
					}}
				>
					{t("devices:models.settings.apply", "Apply")}
				</DvButton>
			</GatedAction>
		</>
	);
}

function SettingsEditor(props: Readonly<EditorProps>) {
	const { t } = useTranslation("devices");
	const editor = useEditor(props);
	const { base } = editor;
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) props.onClose();
			}}
			wide
			icon={editor.reviewing ? Diff : Settings2}
			title={
				editor.reviewing
					? t(
							"devices:models.settings.reviewTitle",
							"Review changes to {{model}}",
							{
								model: base.display_name,
							},
						)
					: t("devices:models.settings.title", "Settings of {{model}}", {
							model: base.display_name,
						})
			}
			sub={t("devices:models.settings.sub", "on {{device}}", {
				device: props.target.name,
			})}
			onBack={editor.reviewing ? () => editor.setReviewing(false) : undefined}
			footNote={
				editor.reviewing
					? t("devices:models.settings.stepApply", "Step 2 of 2 · Apply")
					: t("devices:models.settings.stepChange", "Step 1 of 2 · Change")
			}
			foot={<EditorFoot editor={editor} />}
		>
			{editor.stale ? (
				<StaleBanner onReload={() => void editor.reload()} />
			) : null}
			{editor.reviewing ? (
				<ReviewBody editor={editor} />
			) : (
				<ChangeBody editor={editor} />
			)}
		</DvSheet>
	);
}

const byId = (modelId: string) => (model: HostedModel) => model.id === modelId;

/**
 * Models tab › a hosted model's settings (plan §3.7): every engine setting
 * with its recommended value and why, and when it runs; Configure carries
 * the revision the sheet opened with, so a change made meanwhile is caught.
 */
export function ModelSettingsSheet({
	deviceId,
	modelId,
	scope,
	onClose,
}: Readonly<ModelSettingsSheetProps>) {
	const { t } = useTranslation("devices");
	const target = useObserveTarget(deviceId, null, scope);
	const access = useModelsAccess(deviceId);
	const overview = useModelsOverview(deviceId, { poll: false });
	const listed = overview.data;
	const after = useModelsAfter(deviceId, listed?.next ?? null, {
		poll: false,
	});
	const data = useMemo(
		() => listed && withAllModels(listed, after.data),
		[listed, after.data],
	);
	const model = data?.models.find(byId(modelId));
	const paging = !model && !!listed?.next && after.data === undefined;
	if (data && model)
		return (
			<SettingsEditor
				target={target}
				model={model}
				overview={{ ...overview, data }}
				onClose={onClose}
			/>
		);
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={Settings2}
			title={t("devices:models.settings.titleUnknown", "Model settings")}
			sub={t("devices:models.settings.sub", "on {{device}}", {
				device: target.name,
			})}
			foot={
				<DvButton onClick={onClose}>
					{t("devices:models.settings.close", "Close")}
				</DvButton>
			}
		>
			{data && !paging ? (
				<StateView
					kind="empty"
					title={t(
						"devices:models.settings.gone",
						"This model is no longer on {{device}}.",
						{ device: target.name },
					)}
				/>
			) : (
				<OverviewPending
					target={target}
					access={access}
					overview={
						after.failure
							? { ...overview, failure: after.failure, refetch: after.refetch }
							: overview
					}
				/>
			)}
		</DvSheet>
	);
}
