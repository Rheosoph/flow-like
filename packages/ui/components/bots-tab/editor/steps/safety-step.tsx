"use client";

import { useTranslation } from "@flow-like/locales";
import { GaugeIcon, UsersIcon } from "lucide-react";
import { useEffect, useId, useState } from "react";
import type { IDataSensitivity } from "../../../../lib/data-classification";
import { cn } from "../../../../lib/utils";
import {
	type BotCapability,
	MAX_OTHERS_DAILY_TURNS,
	REQUESTER_DAILY_TURNS,
} from "../../../../state/backend-state/bot-state";
import { DataSensitivityFields } from "../../../ui/data-classification";
import { Input } from "../../../ui/input";
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

function OthersDailyTurnsRow({
	value,
	onChange,
}: Readonly<{ value: number; onChange: (turns: number) => void }>) {
	const { t } = useTranslation("bots");
	const inputId = useId();
	const [text, setText] = useState(String(value));
	useEffect(() => setText(String(value)), [value]);
	const edit = (raw: string) => {
		setText(raw);
		const turns = Number(raw);
		if (
			raw.trim() !== "" &&
			Number.isInteger(turns) &&
			turns >= 1 &&
			turns <= MAX_OTHERS_DAILY_TURNS
		) {
			onChange(turns);
		}
	};
	return (
		<div className="flex items-start gap-3 px-4 py-3.5">
			<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
				<GaugeIcon className="size-4" aria-hidden />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				<Label htmlFor={inputId} className="text-sm font-medium">
					{t(
						"editor.safety.othersDailyTurns",
						"Requests from other people per day",
					)}
				</Label>
				<p
					id={`${inputId}-description`}
					className="text-xs leading-relaxed text-muted-foreground"
				>
					{t(
						"editor.safety.othersDailyTurnsHint",
						"How many turns everyone but you may start on it in a day, in rooms, team threads and chats together; you pay for them. Each person gets at most {{perPerson}}.",
						{ perPerson: REQUESTER_DAILY_TURNS },
					)}
				</p>
			</div>
			<Input
				id={inputId}
				type="number"
				inputMode="numeric"
				min={1}
				max={MAX_OTHERS_DAILY_TURNS}
				aria-describedby={`${inputId}-description`}
				className="w-24 shrink-0"
				value={text}
				onChange={(event) => edit(event.target.value)}
				onBlur={() => setText(String(value))}
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
	onOthersDailyTurnsChange,
}: Readonly<{
	draft: BotDraft;
	/** A FlowPilot bot keeps its abilities, encryption included. */
	locked: boolean;
	/** A bot on this computer only is never seen by anyone else. */
	localOnly: boolean;
	onClassificationChange: (classification: IDataSensitivity) => void;
	onToggle: (capability: BotCapability, enabled: boolean) => void;
	onOthersCanInviteChange: (enabled: boolean) => void;
	onOthersDailyTurnsChange: (turns: number) => void;
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
					<div className="divide-y divide-border rounded-xl border border-border bg-card">
						<OthersCanInviteRow
							checked={draft.othersCanInvite}
							onChange={onOthersCanInviteChange}
						/>
						<OthersDailyTurnsRow
							value={draft.othersDailyTurns}
							onChange={onOthersDailyTurnsChange}
						/>
					</div>
				)}
			</section>
		</BotStepPanel>
	);
}
