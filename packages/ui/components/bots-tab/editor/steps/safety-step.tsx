"use client";

import { useTranslation } from "@flow-like/locales";
import type { IDataSensitivity } from "../../../../lib/data-classification";
import type { BotCapability } from "../../../../state/backend-state/bot-state";
import { DataSensitivityFields } from "../../../ui/data-classification";
import type { BotDraft } from "../bot-draft";
import { BotSectionHeading, BotStepPanel } from "../bot-editor-parts";
import { BotCapabilityRow } from "./abilities-step";

export function SafetyStep({
	draft,
	locked,
	onClassificationChange,
	onToggle,
}: Readonly<{
	draft: BotDraft;
	/** A FlowPilot bot keeps its abilities, encryption included. */
	locked: boolean;
	onClassificationChange: (classification: IDataSensitivity) => void;
	onToggle: (capability: BotCapability, enabled: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<BotStepPanel
			title={t("editor.safety.title")}
			subtitle={t("editor.safety.subtitle")}
		>
			<section className="flex flex-col gap-3">
				<BotSectionHeading>{t("editor.safety.handles")}</BotSectionHeading>
				<DataSensitivityFields
					value={draft.classification}
					onChange={onClassificationChange}
				/>
			</section>
			<section className="flex flex-col gap-3">
				<BotSectionHeading>{t("editor.safety.privacy")}</BotSectionHeading>
				<div className="rounded-xl border border-border bg-card">
					<BotCapabilityRow
						capability="encryption"
						location={draft.location}
						checked={draft.capabilities.includes("encryption")}
						locked={locked}
						onToggle={onToggle}
					/>
				</div>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t("editor.safety.encryptionHint")}
				</p>
			</section>
		</BotStepPanel>
	);
}
