"use client";

import { useTranslation } from "@flow-like/locales";
import { UsersIcon } from "lucide-react";
import { useId } from "react";
import type { IDataSensitivity } from "../../../../lib/data-classification";
import { cn } from "../../../../lib/utils";
import type { BotCapability } from "../../../../state/backend-state/bot-state";
import { DataSensitivityFields } from "../../../ui/data-classification";
import { Label } from "../../../ui/label";
import { Switch } from "../../../ui/switch";
import type { BotDraft } from "../bot-draft";
import { BotSectionHeading, BotStepPanel } from "../bot-editor-parts";
import { BotCapabilityRow } from "./abilities-step";

function OthersCanInviteRow({
	checked,
	onChange,
}: Readonly<{ checked: boolean; onChange: (enabled: boolean) => void }>) {
	const { t } = useTranslation("bots");
	const switchId = useId();
	return (
		<div className="flex items-start gap-3 px-4 py-3.5">
			<span
				className={cn(
					"grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground",
					checked && "bg-primary/10 text-primary",
				)}
			>
				<UsersIcon className="size-4" aria-hidden />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				<Label htmlFor={switchId} className="text-sm font-medium">
					{t("editor.safety.othersCanInvite")}
				</Label>
				<p
					id={`${switchId}-description`}
					className="text-xs leading-relaxed text-muted-foreground"
				>
					{t("editor.safety.othersCanInviteHint")}
				</p>
			</div>
			<Switch
				id={switchId}
				aria-describedby={`${switchId}-description`}
				checked={checked}
				onCheckedChange={onChange}
			/>
		</div>
	);
}

export function SafetyStep({
	draft,
	locked,
	localOnly,
	onClassificationChange,
	onToggle,
	onOthersCanInviteChange,
}: Readonly<{
	draft: BotDraft;
	/** A FlowPilot bot keeps its abilities, encryption included. */
	locked: boolean;
	/** A bot on this computer only is never seen by anyone else. */
	localOnly: boolean;
	onClassificationChange: (classification: IDataSensitivity) => void;
	onToggle: (capability: BotCapability, enabled: boolean) => void;
	onOthersCanInviteChange: (enabled: boolean) => void;
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
				{!localOnly && (
					<div className="rounded-xl border border-border bg-card">
						<OthersCanInviteRow
							checked={draft.othersCanInvite}
							onChange={onOthersCanInviteChange}
						/>
					</div>
				)}
			</section>
		</BotStepPanel>
	);
}
