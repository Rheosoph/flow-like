"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlignLeftIcon,
	AppWindowIcon,
	BotIcon,
	ChevronDownIcon,
	CodeIcon,
	CpuIcon,
	FileTextIcon,
	HardDriveIcon,
	type LucideIcon,
	MapPinIcon,
	ShieldCheckIcon,
	ShieldIcon,
	SlidersHorizontalIcon,
	SmileIcon,
	SparklesIcon,
	UnlinkIcon,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { useAccountDevices } from "../../hooks/use-account-devices";
import { useBotLabels, useBots } from "../../hooks/use-bots";
import { parseDataClassification } from "../../lib/data-classification";
import { cn } from "../../lib/utils";
import {
	BOT_LOCATIONS,
	type BotCapability,
	type BotLocation,
	type BotPermission,
	type BotRestrictionPreset,
} from "../../state/backend-state/bot-state";
import { BotAvatar } from "../bots/bot-avatar";
import { DataClassificationBadge } from "../ui/data-classification";
import {
	useAppNames,
	usePermissionLabels,
	usePresetLabels,
} from "./bot-app-restrictions";
import { useBotCapabilityLabel } from "./bot-labels";
import { BOT_CAPABILITY_ICONS } from "./editor/bot-capability-groups";
import { useBotLocationCopy } from "./editor/bot-editor-copy";

const FIELDS: Record<string, { label: string; icon: LucideIcon }> = {
	name: { label: "Name", icon: BotIcon },
	description: { label: "Description", icon: AlignLeftIcon },
	instructions: { label: "System prompt", icon: FileTextIcon },
	capabilities: { label: "Capabilities", icon: SparklesIcon },
	model: { label: "Chat models", icon: CpuIcon },
	embedding_model: { label: "Embedding model", icon: CpuIcon },
	stt_model: { label: "Speech recognition model", icon: CpuIcon },
	tts_model: { label: "Speech model", icon: CpuIcon },
	image_model: { label: "Image model", icon: CpuIcon },
	vision_model: { label: "Vision model", icon: CpuIcon },
	subtask_model: { label: "Subtask model", icon: CpuIcon },
	look: { label: "Look", icon: SmileIcon },
	classification: { label: "Data classification", icon: ShieldIcon },
	location: { label: "Runs on", icon: MapPinIcon },
	execution_mode: { label: "Execution environment", icon: ShieldCheckIcon },
	device_id: { label: "Device", icon: HardDriveIcon },
	app_ids: { label: "Apps", icon: AppWindowIcon },
	app_restrictions: {
		label: "What it may do in each app",
		icon: ShieldCheckIcon,
	},
};

export type BotSettingsTool =
	| "bot_update_configuration"
	| "create_bot"
	| "update_bot"
	| "delete_bot";

/**
 * Titles and answers for approving a bot's settings change (its own, a new bot, or another bot)
 * or the deletion of another bot.
 */
export function useBotSettingsApprovalText(tool: string | undefined) {
	const { t } = useTranslation("bots");
	if (tool === "delete_bot")
		return {
			title: t("crew.approval.deleteBot.title", "Delete a bot"),
			requestTitle: (name: string) =>
				t(
					"crew.approval.deleteBot.requestTitle",
					"{{name}} wants to delete a bot",
					{
						name,
					},
				),
			hint: t(
				"crew.approval.deleteBot.hint",
				"The bot below is deleted for good with its settings, app access, secrets and workspace files and tables; its past messages stay.",
			),
			approve: t("crew.approval.deleteBot.approve", "Delete bot"),
			reject: t("crew.approval.deleteBot.reject", "Keep bot"),
			approved: t("crew.approval.deleteBot.approved", "Deletion approved"),
			rejected: t("crew.approval.deleteBot.rejected", "Deletion rejected"),
		};
	if (tool === "create_bot")
		return {
			title: t("crew.approval.createBot.title", "Create a bot"),
			requestTitle: (name: string) =>
				t(
					"crew.approval.createBot.requestTitle",
					"{{name}} wants to create a bot",
					{
						name,
					},
				),
			hint: t(
				"crew.approval.createBot.hint",
				"The bot is created with the settings below. You can change it in the Bots tab later.",
			),
			approve: t("crew.approval.createBot.approve", "Create bot"),
			reject: t("crew.approval.createBot.reject", "Don't create"),
			approved: t("crew.approval.createBot.approved", "Bot creation approved"),
			rejected: t("crew.approval.createBot.rejected", "Bot creation rejected"),
		};
	const changes = {
		approve: t("crew.approval.configuration.approve", "Approve changes"),
		reject: t("crew.approval.configuration.reject", "Reject changes"),
		approved: t("crew.approval.configuration.approved", "Changes approved"),
		rejected: t("crew.approval.configuration.rejected", "Changes rejected"),
	};
	if (tool === "update_bot")
		return {
			...changes,
			title: t("crew.approval.updateBot.title", "Change a bot's settings"),
			requestTitle: (name: string) =>
				t(
					"crew.approval.updateBot.requestTitle",
					"{{name}} wants to change a bot's settings",
					{ name },
				),
			hint: t(
				"crew.approval.updateBot.hint",
				"Only the settings below will change, starting with the bot's next turn.",
			),
		};
	return {
		...changes,
		title: t("crew.approval.configuration.title", "Update bot settings"),
		requestTitle: (name: string) =>
			t(
				"crew.approval.configuration.requestTitle",
				"{{name}} wants to update its settings",
				{ name },
			),
		hint: t(
			"crew.approval.configuration.hint",
			"Only the settings below will change, starting with the next turn.",
		),
	};
}

/** The bot an `update_bot` approval changes, by name and look. */
function TargetBot({ botId }: { botId: string }) {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const bot = useBots().data?.find((candidate) => candidate.id === botId);
	return (
		<div
			className="flex min-w-0 items-center gap-2.5 rounded-xl border border-border/60 bg-background/60 px-3 py-2"
			data-slot="bot-configuration-target"
		>
			{bot && (
				<BotAvatar
					appearance={bot.appearance}
					size={28}
					animated={false}
					decorative
				/>
			)}
			<div className="min-w-0">
				<p className="text-xs text-muted-foreground">
					{t("crew.approval.updateBot.target", "Bot")}
				</p>
				<p className="truncate text-sm font-medium">
					{(bot && labels(bot).name) || botId}
				</p>
			</div>
		</div>
	);
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function humanize(field: string): string {
	const words = field.replace(/[_-]/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2");
	return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Keep the full proposed text available without letting a long prompt bury other changes. */
function ProposedText({ value }: { value: string }) {
	const { t } = useTranslation("bots");
	const [expanded, setExpanded] = useState(false);
	const id = useId();
	const long = value.length > 320 || value.split("\n").length > 5;
	return (
		<div className="min-w-0">
			<p
				id={id}
				className={cn(
					"whitespace-pre-wrap text-sm leading-relaxed [overflow-wrap:anywhere]",
					long && !expanded && "line-clamp-4",
				)}
			>
				{value || t("crew.approval.configuration.emptyText", "Empty text")}
			</p>
			{long && (
				<button
					type="button"
					aria-expanded={expanded}
					aria-controls={id}
					onClick={() => setExpanded(!expanded)}
					className="mt-2 inline-flex min-h-8 items-center gap-1.5 rounded-md text-xs font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{expanded
						? t("crew.approval.configuration.collapse", "Show less")
						: t("crew.approval.configuration.expand", "Show full text")}
					<ChevronDownIcon
						aria-hidden
						className={cn(
							"size-3.5 transition-transform",
							expanded && "rotate-180",
						)}
					/>
				</button>
			)}
		</div>
	);
}

function ClearedValue({ label }: { label: string }) {
	return (
		<span className="inline-flex items-center gap-2 text-sm text-muted-foreground">
			<UnlinkIcon className="size-3.5 shrink-0" aria-hidden />
			{label}
		</span>
	);
}

/** Unknown fields still expose every value, including nested lists, without JSON syntax. */
function ProposedValue({ value }: { value: unknown }) {
	const { t } = useTranslation("bots");
	if (value === null || value === undefined)
		return (
			<ClearedValue label={t("crew.approval.clearValue", "Clear value")} />
		);
	if (typeof value === "string")
		return <ProposedText key={value} value={value} />;
	if (typeof value === "boolean")
		return (
			<span className="text-sm">
				{value
					? t("crew.approval.configuration.enabled", "Enabled")
					: t("crew.approval.configuration.disabled", "Disabled")}
			</span>
		);
	if (Array.isArray(value)) {
		if (!value.length)
			return (
				<span className="text-sm text-muted-foreground">
					{t("crew.approval.configuration.emptyList", "Empty list")}
				</span>
			);
		return (
			<ol className="space-y-2">
				{value.map((item, index) => (
					// biome-ignore lint/suspicious/noArrayIndexKey: preserve the immutable proposal's order and duplicates
					<li key={index} className="flex min-w-0 items-start gap-2">
						<span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded bg-muted text-[11px] tabular-nums text-muted-foreground">
							{index + 1}
						</span>
						<div className="min-w-0 flex-1">
							<ProposedValue value={item} />
						</div>
					</li>
				))}
			</ol>
		);
	}
	if (isRecord(value)) {
		if (!Object.keys(value).length)
			return (
				<span className="text-sm text-muted-foreground">
					{t("crew.approval.configuration.emptyObject", "Empty object")}
				</span>
			);
		return (
			<dl className="space-y-3 border-l-2 border-border/60 pl-3">
				{Object.entries(value).map(([key, item]) => (
					<div key={key} className="min-w-0">
						<dt className="mb-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
							{humanize(key)}
						</dt>
						<dd>
							<ProposedValue value={item} />
						</dd>
					</div>
				))}
			</dl>
		);
	}
	return (
		<span className="text-sm [overflow-wrap:anywhere]">{String(value)}</span>
	);
}

function ProposedCapabilities({ values }: { values: string[] }) {
	const { t } = useTranslation("bots");
	const capabilityLabel = useBotCapabilityLabel();
	return (
		<div className="space-y-2.5">
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"crew.approval.configuration.capabilitiesHint",
					"Replaces the current capability list.",
				)}
			</p>
			{values.length ? (
				<ul className="flex flex-wrap gap-1.5">
					{values.map((capability, index) => {
						const Icon =
							BOT_CAPABILITY_ICONS[capability as BotCapability] ?? SparklesIcon;
						return (
							<li
								// biome-ignore lint/suspicious/noArrayIndexKey: preserve duplicate entries in the exact proposal
								key={`${capability}-${index}`}
								className="inline-flex max-w-full items-center gap-1.5 rounded-lg border border-primary/15 bg-primary/5 px-2.5 py-1.5 text-xs font-medium"
							>
								<Icon className="size-3.5 shrink-0 text-primary" aria-hidden />
								<span className="min-w-0 [overflow-wrap:anywhere]">
									{capabilityLabel(capability as BotCapability)}
								</span>
							</li>
						);
					})}
				</ul>
			) : (
				<ClearedValue
					label={t(
						"crew.approval.configuration.noCapabilities",
						"Remove all capabilities",
					)}
				/>
			)}
		</div>
	);
}

function ProposedModels({ value }: { value: Record<string, unknown> }) {
	const { t } = useTranslation("bots");
	const labels: Record<string, string> = {
		primary: t("crew.approval.configuration.primary", "Primary model"),
		decision: t("crew.approval.configuration.decision", "Decision model"),
		fallbacks: t("crew.approval.configuration.fallbacks", "Fallback models"),
	};
	return (
		<dl className="divide-y divide-border/50">
			{Object.entries(value).map(([role, model]) => (
				<div
					key={role}
					className="min-w-0 space-y-1.5 py-2.5 first:pt-0 last:pb-0"
				>
					<dt className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
						{labels[role] ?? humanize(role)}
					</dt>
					<dd>
						{model === null ? (
							<ClearedValue
								label={
									role === "decision"
										? t(
												"crew.approval.configuration.clearDecision",
												"Disable decision model",
											)
										: t(
												"crew.approval.configuration.clearModel",
												"Remove model assignment",
											)
								}
							/>
						) : role === "fallbacks" &&
							Array.isArray(model) &&
							model.length === 0 ? (
							<ClearedValue
								label={t(
									"crew.approval.configuration.noFallbacks",
									"Remove all fallback models",
								)}
							/>
						) : (
							<ProposedValue value={model} />
						)}
					</dd>
				</div>
			))}
		</dl>
	);
}

const SCOPE_LISTS = ["tables", "events", "boards"] as const;

function ProposedClassification({ value }: { value: Record<string, unknown> }) {
	const { t } = useTranslation("bots");
	const level =
		typeof value.classification === "string"
			? parseDataClassification(value.classification)
			: undefined;
	return (
		<dl className="space-y-2.5">
			{"classification" in value && (
				<div className="space-y-1">
					<dt className="text-xs text-muted-foreground">
						{t("crew.approval.classification.level", "Sensitivity")}
					</dt>
					<dd>
						{level ? (
							<DataClassificationBadge classification={level} />
						) : (
							<ClearedValue
								label={t(
									"crew.approval.classification.unset",
									"Not classified",
								)}
							/>
						)}
					</dd>
				</div>
			)}
			{"pii" in value && (
				<div className="space-y-1">
					<dt className="text-xs text-muted-foreground">
						{t("crew.approval.classification.pii", "Personal data")}
					</dt>
					<dd className="text-sm">
						{value.pii === true
							? t(
									"crew.approval.classification.piiYes",
									"Handles personal data",
								)
							: t("crew.approval.classification.piiNo", "No personal data")}
					</dd>
				</div>
			)}
		</dl>
	);
}

function ProposedLocation({ location }: { location: BotLocation }) {
	const copy = useBotLocationCopy()(location);
	return (
		<div className="min-w-0 space-y-0.5">
			<p className="text-sm font-medium">{copy.label}</p>
			<p className="text-xs text-muted-foreground">{copy.description}</p>
		</div>
	);
}

/** A device by its name, as the device picker of the Bots tab shows it. */
function ProposedDevice({ deviceId }: { deviceId: string }) {
	const devices = useAccountDevices().devices ?? [];
	let device: (typeof devices)[number] | undefined;
	for (const candidate of devices)
		if (candidate.device_id === deviceId) device = candidate;
	return (
		<span className="text-sm [overflow-wrap:anywhere]">
			{device?.display_name || device?.name || deviceId}
		</span>
	);
}

/** The preset, the listed permissions, or the whole role of the owner an entry leaves the bot. */
const restrictionRole = (
	entry: Record<string, unknown>,
	presets: ReturnType<typeof usePresetLabels>,
	permissions: ReturnType<typeof usePermissionLabels>,
) => {
	if (typeof entry.preset === "string")
		return presets[entry.preset as BotRestrictionPreset] ?? entry.preset;
	if (!Array.isArray(entry.permissions)) return presets.full;
	const label = (id: unknown) => permissions[id as BotPermission] ?? String(id);
	return entry.permissions.map(label).join(", ");
};

/** The folders, the read-only switch and the allow-lists of one entry. */
function RestrictionScopes({ entry }: { entry: Record<string, unknown> }) {
	const { t } = useTranslation("bots");
	const listLabels = {
		tables: t("restrictions.groups.tables", "Tables"),
		events: t("restrictions.groups.events", "Events"),
		boards: t("restrictions.groups.boards", "Boards"),
	};
	const listed = (values: unknown[]) =>
		values.length
			? values.map(String).join(", ")
			: t("crew.approval.configuration.emptyList", "Empty list");
	const lines = SCOPE_LISTS.filter((list) => Array.isArray(entry[list]));
	return (
		<>
			{Array.isArray(entry.folders) && entry.folders.length > 0 && (
				<p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
					{t("restrictions.foldersOnly", "Only these folders")}:{" "}
					{listed(entry.folders)}
				</p>
			)}
			{entry.files_read_only === true && (
				<p className="text-xs text-muted-foreground">
					{t("restrictions.readOnlyFiles", "Never change files")}
				</p>
			)}
			{lines.map((list) => (
				<p
					key={list}
					className="text-xs text-muted-foreground [overflow-wrap:anywhere]"
				>
					{t("restrictions.limitedTo", "Only these {{what}}", {
						what: listLabels[list].toLocaleLowerCase(),
					})}
					: {listed(entry[list] as unknown[])}
				</p>
			))}
		</>
	);
}

/** Each app by name with the preset or permissions and the scopes the bot gets there. */
function ProposedRestrictions({ entries }: { entries: unknown[] }) {
	const names = useAppNames();
	const presets = usePresetLabels();
	const permissions = usePermissionLabels();
	return (
		<ul className="space-y-3">
			{entries.filter(isRecord).map((entry, index) => {
				const appId = String(entry.app_id ?? "");
				return (
					<li
						// biome-ignore lint/suspicious/noArrayIndexKey: keep the order of the immutable proposal
						key={`${appId}-${index}`}
						className="min-w-0 space-y-1"
						data-app={appId}
					>
						<p className="truncate text-sm font-medium">
							{names.get(appId) || appId}
						</p>
						<p className="text-xs [overflow-wrap:anywhere]">
							{restrictionRole(entry, presets, permissions)}
						</p>
						<RestrictionScopes entry={entry} />
					</li>
				);
			})}
		</ul>
	);
}

function isBotLocation(value: unknown): value is BotLocation {
	return BOT_LOCATIONS.includes(value as BotLocation);
}

/** Crew fields with a view of their own; each returns `null` for a value it can't show. */
const CREW_FIELD_VIEWS: Record<string, (value: unknown) => ReactNode> = {
	classification: (value) =>
		isRecord(value) ? <ProposedClassification value={value} /> : null,
	location: (value) =>
		isBotLocation(value) ? <ProposedLocation location={value} /> : null,
	device_id: (value) =>
		typeof value === "string" ? <ProposedDevice deviceId={value} /> : null,
	app_restrictions: (value) =>
		Array.isArray(value) ? <ProposedRestrictions entries={value} /> : null,
};

function ChangeValue({ field, value }: { field: string; value: unknown }) {
	const { t } = useTranslation("bots");
	const view = CREW_FIELD_VIEWS[field]?.(value);
	if (view) return view;
	if (
		field === "capabilities" &&
		Array.isArray(value) &&
		value.every((item) => typeof item === "string")
	)
		return <ProposedCapabilities values={value} />;
	if (field === "model" && isRecord(value) && Object.keys(value).length)
		return <ProposedModels value={value} />;
	if (field.endsWith("_model") && value === null)
		return (
			<ClearedValue
				label={t(
					"crew.approval.configuration.clearModel",
					"Remove model assignment",
				)}
			/>
		);
	return <ProposedValue value={value} />;
}

/**
 * Review the exact proposed settings; approvals do not carry a snapshot of previous values. A
 * `delete_bot` approval shows only the bot it deletes.
 */
export function BotConfigurationChanges({
	changes,
	tool,
}: { changes: Record<string, unknown>; tool?: string }) {
	const { t } = useTranslation("bots");
	const titleId = useId();
	const { bot_id: target, ...settings } = changes;
	const entries = Object.entries(settings);
	if (tool === "delete_bot" && typeof target === "string")
		return (
			<section className="min-w-0" data-slot="bot-deletion">
				<TargetBot botId={target} />
			</section>
		);
	return (
		<section
			className="min-w-0 space-y-3"
			data-slot="bot-configuration-changes"
			aria-labelledby={titleId}
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h4 id={titleId} className="text-xs font-medium text-muted-foreground">
					{t("crew.approval.configurationChanges", "Proposed changes")}
				</h4>
				<span className="rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium tabular-nums text-muted-foreground">
					{t(
						"crew.approval.configuration.count",
						entries.length === 1 ? "{{count}} setting" : "{{count}} settings",
						{ count: entries.length },
					)}
				</span>
			</div>
			{typeof target === "string" && <TargetBot botId={target} />}
			{entries.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t("crew.approval.configuration.noChanges", "No settings to change")}
				</p>
			) : (
				<dl className="space-y-2">
					{entries.map(([field, value]) => {
						const meta = FIELDS[field];
						const Icon = meta?.icon ?? SlidersHorizontalIcon;
						return (
							<div
								key={field}
								data-field={field}
								className="min-w-0 overflow-hidden rounded-xl border border-border/60 bg-background/60"
							>
								<dt className="flex min-w-0 items-center gap-2 border-b border-border/40 bg-muted/30 px-3 py-2 text-xs font-medium">
									<Icon
										className="size-3.5 shrink-0 text-muted-foreground"
										aria-hidden
									/>
									<span className="min-w-0 [overflow-wrap:anywhere]">
										{String(
											t(
												`crew.approval.fields.${field}`,
												meta?.label ?? humanize(field),
											),
										)}
									</span>
								</dt>
								<dd className="min-w-0 px-3 py-3">
									<ChangeValue field={field} value={value} />
								</dd>
							</div>
						);
					})}
				</dl>
			)}
			<details className="group min-w-0 text-xs text-muted-foreground">
				<summary className="flex min-h-8 w-fit cursor-pointer list-none items-center gap-1.5 rounded-md hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
					<CodeIcon className="size-3.5" aria-hidden />
					{t(
						"crew.approval.configuration.technicalDetails",
						"Technical details",
					)}
					<ChevronDownIcon
						className="size-3.5 transition-transform group-open:rotate-180"
						aria-hidden
					/>
				</summary>
				<pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-lg border border-border/60 bg-muted/40 p-3 text-[11px] leading-relaxed [overflow-wrap:anywhere]">
					{JSON.stringify(changes, null, 2)}
				</pre>
			</details>
		</section>
	);
}
