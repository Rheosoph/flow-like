"use client";

import { useTranslation } from "@flow-like/locales";
import { Hourglass, Lock } from "lucide-react";
import { type ReactNode, useId, useMemo } from "react";
import { cn } from "../../../../lib/utils";
import {
	type BotCapability,
	type BotLocation,
	FLOWPILOT_CAPABILITY,
	isBotCapabilityAvailable,
} from "../../../../state/backend-state/bot-state";
import { Label } from "../../../ui/label";
import { Switch } from "../../../ui/switch";
import {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	BOT_RUNTIME_PENDING_CAPABILITIES,
	type BotCapabilityGroupId,
} from "../bot-capability-groups";
import { BotEmailAddress } from "../../bot-mail";
import type { BotDraft } from "../bot-draft";
import { useBotCapabilityCopy } from "../bot-editor-copy";
import { BotSectionHeading, BotStepPanel, BotTag } from "../bot-editor-parts";
import {
	AttachedAppPicker,
	BotProfilePicker,
	PendingAppPicker,
} from "./app-pickers";
import { BotMemoryModelField } from "./memory-model-field";

function useGroupLabels(): Record<BotCapabilityGroupId, string> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			web: t("editor.abilities.groups.web"),
			apps: t("editor.abilities.groups.apps"),
			hands: t("editor.abilities.groups.hands"),
			talk: t("editor.abilities.groups.talk"),
			memory: t("editor.abilities.groups.memory"),
			security: t("editor.abilities.groups.security"),
		}),
		[t],
	);
}

/** Flux and its duplicates: FlowPilot's complete toolset, as one ability that stays on. */
export function FlowPilotAbilityStep() {
	const { t } = useTranslation("bots");
	const copy = useBotCapabilityCopy()(FLOWPILOT_CAPABILITY);
	const Icon = BOT_CAPABILITY_ICONS[FLOWPILOT_CAPABILITY];
	return (
		<BotStepPanel
			title={t("editor.abilities.flowpilot.title")}
			subtitle={t("editor.abilities.flowpilot.subtitle")}
		>
			<div className="flex items-start gap-3 rounded-xl border border-primary/40 bg-primary/5 px-4 py-3.5">
				<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary">
					<Icon className="size-4" aria-hidden />
				</span>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<div className="flex flex-wrap items-center gap-1.5">
						<span className="text-sm font-medium">{copy.label}</span>
						<BotTag className="text-[0.625rem]">
							<Lock className="size-3" aria-hidden />
							{t("editor.abilities.flowpilot.badge")}
						</BotTag>
					</div>
					<p className="text-xs leading-relaxed text-muted-foreground">
						{copy.description}
					</p>
				</div>
			</div>
		</BotStepPanel>
	);
}

export function AbilitiesStep({
	draft,
	botId,
	pendingAppIds,
	onPendingAppIdsChange,
	onToggle,
	onProfileChange,
	onEmbeddingModelChange,
}: Readonly<{
	draft: BotDraft;
	/** Set in edit mode: app picks are saved at once. */
	botId: string | null;
	pendingAppIds: readonly string[];
	onPendingAppIdsChange: (ids: string[]) => void;
	onToggle: (capability: BotCapability, enabled: boolean) => void;
	onProfileChange: (profileId: string) => void;
	onEmbeddingModelChange: (embeddingModel: string | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const groupLabels = useGroupLabels();
	const profileId = useId();
	const details: Partial<Record<BotCapability, ReactNode>> = {
		memory: (
			<BotMemoryModelField
				value={draft.embeddingModel}
				onChange={onEmbeddingModelChange}
			/>
		),
		profile_apps: (
			<div className="flex flex-col gap-1.5">
				<Label htmlFor={profileId} className="text-xs">
					{t("editor.abilities.profile")}
				</Label>
				<BotProfilePicker
					id={profileId}
					value={draft.profileId}
					onChange={onProfileChange}
				/>
			</div>
		),
		email: botId ? (
			<BotEmailAddress botId={botId} />
		) : (
			<p className="text-xs text-muted-foreground">
				{t(
					"panel.mail.afterSave",
					"Save the bot first, then pick its address here or in its Details.",
				)}
			</p>
		),
		selected_apps: botId ? (
			<AttachedAppPicker botId={botId} />
		) : (
			<PendingAppPicker
				selected={pendingAppIds}
				onChange={onPendingAppIdsChange}
			/>
		),
	};
	return (
		<BotStepPanel
			title={t("editor.abilities.title")}
			subtitle={t(
				"editor.abilities.subtitleMemoryOn",
				"New bots start with Memory on and everything else off. In an app, the bot never has more rights than you.",
			)}
		>
			{BOT_CAPABILITY_GROUPS.map((group) => (
				<section key={group.id} className="flex flex-col gap-2.5">
					<BotSectionHeading>{groupLabels[group.id]}</BotSectionHeading>
					<div className="divide-y divide-border rounded-xl border border-border bg-card">
						{group.capabilities.map((capability) => (
							<BotCapabilityRow
								key={capability}
								capability={capability}
								location={draft.location}
								checked={draft.capabilities.includes(capability)}
								onToggle={onToggle}
							>
								{details[capability]}
							</BotCapabilityRow>
						))}
					</div>
				</section>
			))}
		</BotStepPanel>
	);
}

/** One capability with its switch; `children` show while it is on. */
export function BotCapabilityRow({
	capability,
	location,
	checked,
	locked = false,
	onToggle,
	children,
}: Readonly<{
	capability: BotCapability;
	location: BotLocation;
	checked: boolean;
	/** The switch can't be used: a FlowPilot bot's abilities are fixed. */
	locked?: boolean;
	onToggle: (capability: BotCapability, enabled: boolean) => void;
	children?: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const copy = useBotCapabilityCopy()(capability);
	const switchId = useId();
	const Icon = BOT_CAPABILITY_ICONS[capability];
	const available = isBotCapabilityAvailable(capability, location);
	const pendingRuntime = BOT_RUNTIME_PENDING_CAPABILITIES.has(capability);
	return (
		<div className="flex flex-col gap-3 px-4 py-3.5">
			<div className="flex items-start gap-3">
				<span
					className={cn(
						"grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground",
						checked && "bg-primary/10 text-primary",
					)}
				>
					<Icon className="size-4" aria-hidden />
				</span>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<div className="flex flex-wrap items-center gap-1.5">
						<Label htmlFor={switchId} className="text-sm font-medium">
							{copy.label}
						</Label>
						{pendingRuntime && (
							<BotTag className="text-[0.625rem]">
								<Hourglass className="size-3" aria-hidden />
								{t("editor.abilities.runtime")}
							</BotTag>
						)}
					</div>
					<p
						id={`${switchId}-description`}
						className="text-xs leading-relaxed text-muted-foreground"
					>
						{copy.description}
						{!available && (
							<span className="mt-1 block text-warning">
								{t("editor.abilities.needsLocal")}
							</span>
						)}
					</p>
				</div>
				<Switch
					id={switchId}
					aria-describedby={`${switchId}-description`}
					checked={checked}
					disabled={locked || (!available && !checked)}
					onCheckedChange={(enabled) => onToggle(capability, enabled)}
				/>
			</div>
			{checked && children && <div className="pl-11">{children}</div>}
		</div>
	);
}
