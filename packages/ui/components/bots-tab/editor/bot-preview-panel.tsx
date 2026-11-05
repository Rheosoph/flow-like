"use client";

import { useTranslation } from "@flow-like/locales";
import { Cloud, Laptop, type LucideIcon, Server } from "lucide-react";
import { useBotLabels } from "../../../hooks/use-bots";
import { cn } from "../../../lib/utils";
import {
	type BotCapability,
	type BotLocation,
	type BotSystemKey,
	FLOWPILOT_CAPABILITY,
} from "../../../state/backend-state/bot-state";
import { BotAvatar } from "../../bots";
import { DataSensitivityBadges } from "../../ui/data-classification";
import { BOT_CAPABILITY_ICONS } from "./bot-capability-groups";
import { type BotDraft, hasFixedLocation, hasFlowPilot } from "./bot-draft";
import { useBotCapabilityCopy, useBotLocationCopy } from "./bot-editor-copy";
import { BotTag } from "./bot-editor-parts";

export const BOT_LOCATION_ICONS: Readonly<Record<BotLocation, LucideIcon>> = {
	server: Cloud,
	desktop: Laptop,
	device: Server,
};

const SUMMARY_LIMIT = 6;

export interface BotIdentityChange {
	name?: string;
	description?: string;
}

/** The bot as it will look, with its name and description editable in place. */
export function BotPreviewPanel({
	draft,
	system,
	readOnly = false,
	badge,
	nameInvalid = false,
	onChange,
	className,
}: Readonly<{
	draft: BotDraft;
	system: BotSystemKey | null;
	readOnly?: boolean;
	badge?: string;
	nameInvalid?: boolean;
	onChange?: (change: BotIdentityChange) => void;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const defaults = useBotLabels()({ system });
	const name = draft.name.trim() || defaults.name;
	return (
		<aside
			aria-label={t("editor.preview.label")}
			className={cn(
				"min-w-0 overflow-hidden rounded-2xl border border-border bg-card",
				className,
			)}
		>
			<div className="flex items-center justify-between gap-2 px-5 pt-5">
				<span className="text-[0.625rem] font-bold tracking-[0.14em] text-muted-foreground uppercase">
					{t("editor.preview.eyebrow")}
				</span>
				{badge && <BotTag className="text-[0.625rem]">{badge}</BotTag>}
			</div>
			<div className="flex min-h-52 items-center justify-center bg-radial from-primary/10 to-transparent to-70% px-6 py-6">
				<BotAvatar
					appearance={draft.appearance}
					size={168}
					label={name || t("editor.preview.unnamed")}
				/>
			</div>
			<div className="flex flex-col gap-1 px-4 pb-4 text-center">
				{readOnly ? (
					<ReadOnlyIdentity
						name={name || t("editor.preview.unnamed")}
						description={draft.description.trim() || defaults.description}
					/>
				) : (
					<EditableIdentity
						draft={draft}
						defaults={defaults}
						nameInvalid={nameInvalid}
						onChange={onChange}
					/>
				)}
			</div>
			<BotPreviewSummary
				draft={draft}
				abilities={
					hasFlowPilot({ system, capabilities: draft.capabilities })
						? [FLOWPILOT_CAPABILITY]
						: draft.capabilities
				}
				showLocation={!hasFixedLocation(system)}
			/>
		</aside>
	);
}

function ReadOnlyIdentity({
	name,
	description,
}: Readonly<{ name: string; description: string }>) {
	return (
		<>
			<h3 className="text-2xl font-semibold tracking-tight wrap-break-word">
				{name}
			</h3>
			<p className="min-h-10 text-xs leading-relaxed text-muted-foreground">
				{description}
			</p>
		</>
	);
}

function EditableIdentity({
	draft,
	defaults,
	nameInvalid,
	onChange,
}: Readonly<{
	draft: BotDraft;
	defaults: { name: string; description: string };
	nameInvalid: boolean;
	onChange?: (change: BotIdentityChange) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<input
				aria-label={t("editor.preview.name")}
				aria-invalid={nameInvalid || undefined}
				value={draft.name}
				placeholder={defaults.name || t("editor.preview.namePlaceholder")}
				onChange={(event) => onChange?.({ name: event.target.value })}
				className="w-full rounded-lg border border-transparent bg-transparent px-2 py-1 text-center text-2xl font-semibold tracking-tight outline-none placeholder:text-muted-foreground/70 hover:border-border focus-visible:border-ring aria-invalid:border-destructive"
			/>
			<textarea
				aria-label={t("editor.preview.description")}
				value={draft.description}
				rows={2}
				placeholder={
					defaults.description || t("editor.preview.descriptionPlaceholder")
				}
				onChange={(event) => onChange?.({ description: event.target.value })}
				className="field-sizing-content min-h-10 w-full resize-none rounded-lg border border-transparent bg-transparent px-2 py-1 text-center text-xs leading-relaxed text-muted-foreground outline-none placeholder:text-muted-foreground/70 hover:border-border focus-visible:border-ring"
			/>
			{defaults.name && (
				<p className="text-[0.6875rem] text-muted-foreground">
					{t("editor.preview.defaultHint", { name: defaults.name })}
				</p>
			)}
		</>
	);
}

/** Where it runs, what it may handle and what it can do. */
function BotPreviewSummary({
	draft,
	abilities,
	showLocation,
}: Readonly<{
	draft: BotDraft;
	abilities: readonly BotCapability[];
	showLocation: boolean;
}>) {
	const { t } = useTranslation("bots");
	const capabilityCopy = useBotCapabilityCopy();
	const locationCopy = useBotLocationCopy();
	const LocationIcon = BOT_LOCATION_ICONS[draft.location];
	const shown = abilities.slice(0, SUMMARY_LIMIT);
	const hidden = abilities.length - shown.length;
	return (
		<div className="flex flex-col gap-3 border-t border-border px-4 py-4">
			<div className="flex flex-wrap items-center justify-center gap-1.5">
				{showLocation && (
					<BotTag>
						<LocationIcon className="size-3" aria-hidden />
						{locationCopy(draft.location).label}
					</BotTag>
				)}
				<DataSensitivityBadges value={draft.classification} />
			</div>
			<div className="flex flex-wrap justify-center gap-1.5">
				{shown.map((capability) => {
					const Icon = BOT_CAPABILITY_ICONS[capability];
					return (
						<BotTag key={capability}>
							<Icon className="size-3" aria-hidden />
							{capabilityCopy(capability).label}
						</BotTag>
					);
				})}
				{hidden > 0 && <BotTag>+{hidden}</BotTag>}
				{abilities.length === 0 && (
					<span className="text-[0.6875rem] text-muted-foreground">
						{t("editor.preview.noCapabilities")}
					</span>
				)}
			</div>
		</div>
	);
}
