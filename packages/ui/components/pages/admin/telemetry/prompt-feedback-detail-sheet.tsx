"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { ChevronDown, Copy, MessageSquareQuote } from "lucide-react";
import { useMemo } from "react";
import { toast } from "sonner";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
	RelativeTime,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";
import { safeStringify } from "./issues-shared";
import {
	PromptFeedbackRatingBadge,
	isFailedOutcome,
} from "./prompt-feedback-shared";
import { EmptyState } from "./telemetry-shared";
import { formatDurationMs } from "./traces-shared";
import type {
	IPromptFeedbackDetail,
	IPromptFeedbackTranscriptEntry,
} from "./types";

type RunContext = Record<string, unknown> | null | undefined;

function readString(context: RunContext, key: string): string | undefined {
	const value = context?.[key];
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

function readNumber(context: RunContext, key: string): number | undefined {
	const value = context?.[key];
	return typeof value === "number" && Number.isFinite(value)
		? value
		: undefined;
}

function readBoolean(context: RunContext, key: string): boolean | undefined {
	const value = context?.[key];
	return typeof value === "boolean" ? value : undefined;
}

function CopyButton({
	label,
	text,
}: {
	readonly label: string;
	readonly text: string;
}) {
	const { t } = useTranslation("admin");
	return (
		<Button
			variant="ghost"
			size="sm"
			onClick={() => {
				navigator.clipboard.writeText(text).catch(() => null);
				toast.success(t("labelCopied", "{{label}} copied", { label }));
			}}
		>
			<Copy className="mr-1 h-3 w-3" />
			{t("copy", "Copy")}
		</Button>
	);
}

function TextBlock({
	label,
	text,
	emptyMessage,
}: {
	readonly label: string;
	readonly text: string;
	readonly emptyMessage: string;
}) {
	return (
		<div className="space-y-1">
			<div className="flex items-center justify-between">
				<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
					{label}
				</div>
				{text ? <CopyButton label={label} text={text} /> : null}
			</div>
			{text ? (
				<pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
					{text}
				</pre>
			) : (
				<EmptyState message={emptyMessage} />
			)}
		</div>
	);
}

function CollapsibleJson({
	label,
	value,
}: {
	readonly label: string;
	readonly value: unknown;
}) {
	const text = useMemo(() => safeStringify(value), [value]);
	return (
		<Collapsible className="space-y-1">
			<div className="flex items-center justify-between">
				<CollapsibleTrigger className="group flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground">
					<ChevronDown className="h-3 w-3 transition-transform group-data-[state=open]:rotate-180" />
					{label}
				</CollapsibleTrigger>
				<CopyButton label={label} text={text} />
			</div>
			<CollapsibleContent>
				<pre className="max-h-72 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
					{text}
				</pre>
			</CollapsibleContent>
		</Collapsible>
	);
}

function OutcomeBadge({ outcome }: { readonly outcome: string }) {
	const failed = isFailedOutcome(outcome);
	return (
		<Badge
			variant={failed ? "destructive" : "outline"}
			className="font-mono text-[10px] uppercase"
		>
			{outcome}
		</Badge>
	);
}

function BadgeRow({
	label,
	values,
}: {
	readonly label: string;
	readonly values: string[];
}) {
	// Repeats carry no extra signal in a summary row, and deduping keeps the keys stable.
	const unique = useMemo(() => Array.from(new Set(values)), [values]);
	if (unique.length === 0) return null;
	return (
		<div className="space-y-1">
			<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
				{label}
			</div>
			<div className="flex flex-wrap gap-1.5">
				{unique.map((value) => (
					<Badge
						key={value}
						variant="secondary"
						className="max-w-full truncate font-mono text-[10px]"
						title={value}
					>
						{value}
					</Badge>
				))}
			</div>
		</div>
	);
}

interface MetaRow {
	readonly key: string;
	readonly label: string;
	readonly value: string;
	readonly tone?: string;
}

function MetaGrid({ rows }: { readonly rows: MetaRow[] }) {
	if (rows.length === 0) return null;
	return (
		<div className="grid grid-cols-2 gap-3 rounded-lg border bg-card/40 p-3 sm:grid-cols-4">
			{rows.map((row) => (
				<div key={row.key} className="min-w-0">
					<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
						{row.label}
					</div>
					<div
						className={`truncate text-xs font-medium ${row.tone ?? ""}`}
						title={row.value}
					>
						{row.value}
					</div>
				</div>
			))}
		</div>
	);
}

function TranscriptTimeline({
	entries,
}: {
	readonly entries: IPromptFeedbackTranscriptEntry[];
}) {
	return (
		<ol className="max-h-96 space-y-2.5 overflow-auto border-l border-border pl-3">
			{entries.map((entry, index) => (
				<li key={`${entry.timestamp}-${index}`} className="relative">
					<span className="absolute -left-[17px] top-1.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
					<div className="flex flex-wrap items-baseline gap-2">
						<Badge
							variant="outline"
							className="font-mono text-[10px] uppercase"
						>
							{entry.role}
						</Badge>
						{entry.timestamp ? (
							<RelativeTime
								value={entry.timestamp}
								className="text-[10px] text-muted-foreground"
							/>
						) : null}
					</div>
					<p className="whitespace-pre-wrap break-words text-xs text-foreground">
						{entry.content}
					</p>
				</li>
			))}
		</ol>
	);
}

interface PromptFeedbackDetailSheetProps {
	feedbackId: string | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	profile: IProfile | undefined;
}

export function PromptFeedbackDetailSheet({
	feedbackId,
	open,
	onOpenChange,
	profile,
}: Readonly<PromptFeedbackDetailSheetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const detail = useQuery<IPromptFeedbackDetail>({
		queryKey: ["admin", "telemetry", "prompt-feedback", "detail", feedbackId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!feedbackId) throw new Error("No feedback selected");
			return backend.apiState.get<IPromptFeedbackDetail>(
				profile,
				`admin/telemetry/prompt-feedback/${encodeURIComponent(feedbackId)}`,
			);
		},
		enabled: Boolean(profile && feedbackId && open),
	});

	const record = detail.data?.record;
	const runContext = detail.data?.runContext;
	const transcript = detail.data?.transcript ?? [];
	const comment = record?.comment?.trim() ?? "";

	const metaRows = useMemo<MetaRow[]>(() => {
		if (!record) return [];
		const rows: MetaRow[] = [];
		const push = (
			key: string,
			label: string,
			value: string | undefined,
			tone?: string,
		) => {
			if (value === undefined || value === "") return;
			rows.push({ key, label, value, tone });
		};
		const flag = (value: boolean | undefined) =>
			value === undefined ? undefined : value ? t("yes", "Yes") : t("no", "No");

		const durationMs =
			record.durationMs ?? readNumber(runContext, "duration_ms");
		const steerCount = readNumber(runContext, "steer_count");

		push(
			"provider",
			t("provider2", "Provider"),
			record.provider ?? readString(runContext, "provider"),
		);
		push(
			"model",
			t("model", "Model"),
			record.model ??
				readString(runContext, "effective_model_id") ??
				readString(runContext, "model_id"),
		);
		push(
			"reasoningEffort",
			t("reasoningEffort", "Reasoning effort"),
			record.reasoningEffort ?? readString(runContext, "reasoning_effort"),
		);
		push(
			"autoMode",
			t("autoMode", "Auto mode"),
			flag(record.autoMode ?? readBoolean(runContext, "auto_mode")),
		);
		push(
			"surface",
			t("surface", "Surface"),
			record.surface ?? readString(runContext, "surface"),
		);
		push("mode", t("mode", "Mode"), readString(runContext, "mode"));
		push(
			"duration",
			t("duration", "Duration"),
			durationMs === undefined || durationMs === null
				? undefined
				: formatDurationMs(durationMs),
		);
		push(
			"totalTokens",
			t("totalTokens", "Total tokens"),
			record.totalTokens == null
				? undefined
				: record.totalTokens.toLocaleString(),
		);
		push(
			"steerCount",
			t("steerCount", "Steer count"),
			steerCount === undefined ? undefined : steerCount.toLocaleString(),
		);
		push(
			"resumed",
			t("resumed", "Resumed"),
			flag(readBoolean(runContext, "resumed")),
		);
		push(
			"terminalCode",
			t("terminalCode", "Terminal code"),
			readString(runContext, "terminal_code"),
		);
		push(
			"error",
			t("error", "Error"),
			readString(runContext, "error"),
			"text-destructive",
		);
		return rows;
	}, [record, runContext, t]);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-3xl lg:max-w-5xl">
				<SheetHeader>
					<SheetTitle className="flex flex-wrap items-center gap-2 pr-8">
						{record ? (
							<PromptFeedbackRatingBadge rating={record.rating} />
						) : null}
						<span className="truncate font-mono text-sm">
							{record?.model ?? t("ratedTurn", "Rated turn")}
						</span>
						{record?.provider ? (
							<Badge variant="outline" className="font-mono text-[10px]">
								{record.provider}
							</Badge>
						) : null}
						{record?.outcome ? <OutcomeBadge outcome={record.outcome} /> : null}
					</SheetTitle>
					<SheetDescription className="flex flex-wrap items-center gap-2 text-xs">
						{record ? (
							<RelativeTime
								value={record.createdAt}
								className="text-muted-foreground"
							/>
						) : null}
						{record?.conversationId ? (
							<code
								className="truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]"
								title={record.conversationId}
							>
								{record.conversationId}
							</code>
						) : null}
					</SheetDescription>
				</SheetHeader>

				<div className="space-y-4 px-4 pb-8">
					{detail.isLoading ? (
						<div className="space-y-3">
							<Skeleton className="h-16 w-full" />
							<Skeleton className="h-24 w-full" />
							<Skeleton className="h-48 w-full" />
						</div>
					) : detail.isError || !record ? (
						<EmptyState
							message={t(
								"thisRatingCouldNotBeLoadedItMayHaveBeenSweptByRetention",
								"This rating could not be loaded — it may have been swept by retention.",
							)}
							className="py-10 text-sm"
						/>
					) : (
						<>
							<MetaGrid rows={metaRows} />

							<div
								className={`space-y-1.5 rounded-lg border p-3 ${comment ? "border-border bg-muted/40" : "border-dashed"}`}
							>
								<div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wide text-muted-foreground">
									<MessageSquareQuote className="h-3.5 w-3.5" />
									{t("reviewerComment", "Reviewer comment")}
								</div>
								{comment ? (
									<p className="whitespace-pre-wrap break-words text-sm text-foreground">
										{comment}
									</p>
								) : (
									<p className="text-xs text-muted-foreground">
										{t(
											"theReviewerRatedThisTurnWithoutLeavingAComment",
											"The reviewer rated this turn without leaving a comment.",
										)}
									</p>
								)}
								{detail.data?.canContact ? (
									<p className="text-[11px] text-muted-foreground">
										{t(
											"theReviewerAgreedToBeContactedAboutThisRating",
											"The reviewer agreed to be contacted about this rating.",
										)}
									</p>
								) : null}
							</div>

							<TextBlock
								label={t("prompt", "Prompt")}
								text={detail.data?.prompt ?? ""}
								emptyMessage={t(
									"noPromptWasCapturedForThisTurn",
									"No prompt was captured for this turn.",
								)}
							/>
							<TextBlock
								label={t("response", "Response")}
								text={detail.data?.response ?? ""}
								emptyMessage={t(
									"noResponseWasCapturedForThisTurn",
									"No response was captured for this turn.",
								)}
							/>

							<BadgeRow
								label={t("steps", "Steps")}
								values={detail.data?.steps ?? []}
							/>
							<BadgeRow
								label={t("tools", "Tools")}
								values={detail.data?.tools ?? []}
							/>
							<BadgeRow
								label={t("referencedApps", "Referenced apps")}
								values={detail.data?.appRefs ?? []}
							/>

							{runContext ? (
								<CollapsibleJson
									label={t("runContext", "Run context")}
									value={runContext}
								/>
							) : (
								<EmptyState
									message={t(
										"noRunContextWasStampedOnThisTurn",
										"No run context was stamped on this turn.",
									)}
								/>
							)}
							{detail.data?.usage ? (
								<CollapsibleJson
									label={t("tokenUsage", "Token usage")}
									value={detail.data.usage}
								/>
							) : null}

							<Separator />

							<div className="space-y-1.5">
								<div className="flex items-center justify-between">
									<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
										{t("conversationTranscript", "Conversation transcript")}
									</div>
									{detail.data?.transcriptTruncated ? (
										<span className="text-[11px] text-muted-foreground">
											{t(
												"onlyTheMostRecentMessagesWereKept",
												"Only the most recent messages were kept.",
											)}
										</span>
									) : null}
								</div>
								{transcript.length > 0 ? (
									<TranscriptTimeline entries={transcript} />
								) : (
									<p className="text-xs text-muted-foreground">
										{t(
											"theReviewerDidNotIncludeTheChatHistoryWithThisRatingSoOnlyTheRatedTurnIsAvailable",
											"The reviewer did not include the chat history with this rating, so only the rated turn is available.",
										)}
									</p>
								)}
							</div>
						</>
					)}
				</div>
			</SheetContent>
		</Sheet>
	);
}
