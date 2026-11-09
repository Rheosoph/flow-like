"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlignLeftIcon,
	BotIcon,
	ChevronDownIcon,
	CodeIcon,
	CpuIcon,
	FileTextIcon,
	type LucideIcon,
	SlidersHorizontalIcon,
	SparklesIcon,
	UnlinkIcon,
} from "lucide-react";
import { useId, useState } from "react";
import { cn } from "../../lib/utils";
import type { BotCapability } from "../../state/backend-state/bot-state";
import { useBotCapabilityLabel } from "./bot-labels";
import { BOT_CAPABILITY_ICONS } from "./editor/bot-capability-groups";

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
};

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

function ChangeValue({ field, value }: { field: string; value: unknown }) {
	const { t } = useTranslation("bots");
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

/** Review the exact proposed settings; approvals do not carry a snapshot of previous values. */
export function BotConfigurationChanges({
	changes,
}: { changes: Record<string, unknown> }) {
	const { t } = useTranslation("bots");
	const titleId = useId();
	const entries = Object.entries(changes);
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
