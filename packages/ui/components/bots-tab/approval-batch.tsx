"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCheckIcon,
	CheckIcon,
	ChevronDownIcon,
	CircleSlashIcon,
	GlobeIcon,
	type LucideIcon,
	ShieldQuestionIcon,
	TimerOffIcon,
} from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useBotSecretSealing } from "../../hooks/use-bot-secret-sealing";
import { useUpsertBotSecret } from "../../hooks/use-bots";
import type { BotSecretSealing } from "../../lib/bot-secret-sealing";
import { cn } from "../../lib/utils";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAppearance } from "../bots/types";
import {
	RequestCard,
	RequestCardBody,
	RequestCardFooter,
	RequestCardHeader,
	RequestDetails,
	RequestMark,
	RequestPill,
	type RequestTone,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import { RadioGroup, RadioGroupItem } from "../ui/radio-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { useApprovalAnswer } from "./approval-answer";
import {
	type AlwaysBlock,
	type BatchAnswer,
	CLOSED_GROUPS,
	type CallGroup,
	type CallText,
	type ChoiceOf,
	FEW_CALLS,
	type Tally,
	alwaysBlock,
	batchAnswer,
	callCount,
	callTexts,
	groupChoice,
	groupItems,
	tallyOf,
} from "./approval-batch-model";
import type { ApprovalDecision } from "./approval-entry";
import type {
	ConversationApproval,
	ConversationApprovalItem,
} from "./fold-entries";
import { useGrantLabel } from "./grant-labels";
import { InviteRequestDetails } from "./invite-request";
import { SecretRequestForm } from "./secret-request-form";

/** The tool a call uses, as the card names it. */
export function ToolChip({
	tool,
	className,
}: Readonly<{ tool: string; className?: string }>) {
	return (
		<code
			className={cn(
				"rounded-md border border-border/60 bg-muted/60 px-1.5 py-0.5 font-mono text-[11px] text-foreground/80",
				className,
			)}
		>
			{tool}
		</code>
	);
}

export function useDecisionText() {
	const { t } = useTranslation("bots");
	return (decision: ApprovalDecision, secret?: boolean): string => {
		if (secret)
			return decision === "deny"
				? t("crew.approval.secret.declined", "Declined")
				: t("crew.approval.secret.saved", "Secret saved");
		return {
			once: t("crew.approval.answeredOnce", "Allowed once"),
			always: t("crew.approval.answeredAlways", "Always allowed"),
			deny: t("crew.approval.answeredDeny", "Denied"),
		}[decision];
	};
}

/**
 * "Asked again", with why on hover and focus. In a list narrower than 24rem it keeps only the globe
 * the header's notice shows; the words stay for screen readers.
 */
export function AskedAgainPill({ target }: Readonly<{ target: string }>) {
	const { t } = useTranslation("bots");
	return (
		<Tooltip>
			<TooltipTrigger
				type="button"
				className="shrink-0 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
			>
				<RequestPill
					tone="warning"
					icon={GlobeIcon}
					className="h-5 px-1.5 leading-5"
				>
					<span className="@max-sm:sr-only">
						{t("crew.approval.webContent.badge", "Asked again")}
					</span>
				</RequestPill>
			</TooltipTrigger>
			<TooltipContent className="max-w-72">
				{t(
					"crew.approval.webContent.hint",
					"You always allow {{target}}, but this turn read web pages first, so it asks once more. Turns without web content still use your permission.",
					{ target },
				)}
			</TooltipContent>
		</Tooltip>
	);
}

/** The header line that says why a call the owner always allows asks again. */
export function WebContentNotice({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	return (
		<p
			data-slot="web-content-notice"
			className={cn("mt-1 flex items-center gap-1.5 text-xs", className)}
		>
			<GlobeIcon className="size-3.5 shrink-0 text-warning" aria-hidden />
			{t(
				"crew.approval.webContent.notice",
				"Asked again because this turn read web pages.",
			)}
		</p>
	);
}

export function RequesterLine({
	approval,
}: Readonly<{ approval: ConversationApproval }>) {
	const { t } = useTranslation("bots");
	if (!approval.requesterName) return null;
	return (
		<p data-slot="approval-requester" className="mt-1 text-xs">
			{approval.room
				? t(
						"crew.approval.triggeredByInRoom",
						"Triggered by {{name}} in {{room}}",
						{ name: approval.requesterName, room: approval.room },
					)
				: t("crew.approval.triggeredBy", "Triggered by {{name}}", {
						name: approval.requesterName,
					})}
		</p>
	);
}

function useCountText() {
	const { t } = useTranslation("bots");
	return (count: number) =>
		t("crew.approval.batch.count", "{{count}} calls", {
			count,
			defaultValue_one: "{{count}} call",
		});
}

function useBlockText() {
	const { t } = useTranslation("bots");
	return (block: AlwaysBlock | undefined): string | undefined => {
		if (block === "webContent")
			return t(
				"crew.approval.webContent.noAlways",
				"Can't be remembered after the turn read web pages",
			);
		if (block === "notRememberable")
			return t(
				"crew.approval.batch.notRememberable",
				"This call is asked every time",
			);
		return undefined;
	};
}

function DecisionPill({
	decision,
	secret,
}: Readonly<{ decision: ApprovalDecision | undefined; secret?: boolean }>) {
	const text = useDecisionText();
	if (!decision) return null;
	const denied = decision === "deny";
	return (
		<RequestPill
			tone={denied ? "critical" : "good"}
			icon={denied ? BanIcon : CheckIcon}
			className="ml-auto h-5 shrink-0 px-1.5 leading-5"
		>
			{text(decision, secret)}
		</RequestPill>
	);
}

const OPTIONS: readonly ApprovalDecision[] = ["once", "always", "deny"];

/** Once, Always or Deny for one call or a whole group, as one horizontal radio group. */
function ItemDecision({
	value,
	onChange,
	label,
	alwaysBlocked,
	disabled,
}: Readonly<{
	value: ApprovalDecision | undefined;
	onChange: (decision: ApprovalDecision) => void;
	label: string;
	alwaysBlocked?: string;
	disabled?: boolean;
}>) {
	const { t } = useTranslation("bots");
	const id = useId();
	const names: Record<ApprovalDecision, string> = {
		once: t("crew.approval.batch.optionOnce", "Once"),
		always: t("crew.approval.batch.optionAlways", "Always"),
		deny: t("crew.approval.batch.optionDeny", "Deny"),
	};
	return (
		<RadioGroup
			value={value ?? ""}
			onValueChange={(next) => onChange(next as ApprovalDecision)}
			orientation="horizontal"
			loop
			disabled={disabled}
			aria-label={label}
			data-slot="item-decision"
			className="ml-auto inline-flex shrink-0 gap-0.5 rounded-lg border border-border/70 bg-muted/50 p-0.5"
		>
			{OPTIONS.map((option) => {
				const blocked = option === "always" ? alwaysBlocked : undefined;
				const checked = value === option;
				return (
					<label
						key={option}
						htmlFor={`${id}-${option}`}
						title={blocked}
						data-checked={checked || undefined}
						className={cn(
							"flex h-6 cursor-pointer items-center gap-1.5 rounded-md px-2 text-[11px] leading-6 font-medium text-muted-foreground transition-colors hover:text-foreground",
							checked && "bg-card text-foreground ring-1 ring-border",
							checked && option === "deny" && "text-critical",
							(blocked || disabled) &&
								"cursor-not-allowed opacity-50 hover:text-muted-foreground",
						)}
					>
						<RadioGroupItem
							id={`${id}-${option}`}
							value={option}
							disabled={blocked !== undefined}
							aria-describedby={blocked ? `${id}-why` : undefined}
							className="size-3.5 bg-card"
						/>
						{names[option]}
					</label>
				);
			})}
			{alwaysBlocked && (
				<span id={`${id}-why`} className="sr-only">
					{alwaysBlocked}
				</span>
			)}
		</RadioGroup>
	);
}

/** How the list shows each call: as a line, with its choice, or with its answer. */
export interface ListMode {
	calls: boolean;
	/** Marks calls asked again after web content; only while the card waits. */
	flags: boolean;
	choose?: {
		choiceOf: ChoiceOf;
		onChoose: (
			items: readonly ConversationApprovalItem[],
			decision: ApprovalDecision,
		) => void;
		disabled: boolean;
	};
	answerOf?: ChoiceOf;
}

function BatchItemRow({
	item,
	text,
	mode,
	decides,
}: Readonly<{
	item: ConversationApprovalItem;
	text: CallText;
	mode: ListMode;
	/** The row carries its own choice; false when its group's row already does. */
	decides: boolean;
}>) {
	const { t } = useTranslation("bots");
	const countText = useCountText();
	const blockText = useBlockText();
	const [open, setOpen] = useState(false);
	const line = text.full;
	const choose = decides ? mode.choose : undefined;
	return (
		<li className="flex flex-wrap items-center gap-x-2 gap-y-1">
			<button
				type="button"
				aria-expanded={open}
				aria-label={line}
				title={line}
				onClick={() => setOpen((value) => !value)}
				className="flex min-w-0 grow basis-48 rounded text-left text-xs leading-5 text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40"
			>
				{text.start && <span className="min-w-0 truncate">{text.start}</span>}
				{text.end && (
					<span className="shrink-0 whitespace-pre tabular-nums">
						{text.end}
					</span>
				)}
			</button>
			{item.calls > 1 && (
				<span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
					{countText(item.calls)}
				</span>
			)}
			{choose && (
				<ItemDecision
					value={choose.choiceOf(item)}
					onChange={(decision) => choose.onChoose([item], decision)}
					label={t("crew.approval.batch.callAnswer", "Answer for {{summary}}", {
						summary: line,
					})}
					alwaysBlocked={blockText(alwaysBlock(item))}
					disabled={choose.disabled}
				/>
			)}
			{mode.answerOf && <DecisionPill decision={mode.answerOf(item)} />}
			{item.invite && !open && (
				<p
					data-slot="approval-invite-reason"
					title={item.invite.reason}
					className="basis-full line-clamp-2 text-xs text-muted-foreground"
				>
					{t("crew.approval.invite.because", "Why: {{reason}}", {
						reason: item.invite.reason,
					})}
				</p>
			)}
			{open && (
				<div className="basis-full pb-1">
					{item.invite ? (
						<InviteRequestDetails invite={item.invite} />
					) : (
						<RequestDetails text={item.summary || item.tool} />
					)}
				</div>
			)}
		</li>
	);
}

function BatchGroup({
	group,
	mode,
}: Readonly<{ group: CallGroup; mode: ListMode }>) {
	const { t } = useTranslation("bots");
	const grantLabel = useGrantLabel(group.scopes);
	const countText = useCountText();
	const blockText = useBlockText();
	const target =
		group.scopes.length > 0
			? group.scopes.map((scope) => grantLabel(group.tool, scope)).join(", ")
			: t("crew.approval.askEveryTime", "Always asks first");
	const choose = mode.choose;
	const choice = choose ? groupChoice(group, choose.choiceOf) : undefined;
	const lone = group.items.length === 1;
	const block = group.items.map(alwaysBlock).find(Boolean);
	const textOf = useMemo(() => callTexts(group.items), [group.items]);
	return (
		<li data-slot="approval-group" className="py-2.5 first:pt-2 last:pb-2">
			<div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
				<p className="flex min-w-0 grow basis-56 items-center gap-1.5 text-xs">
					<ToolChip tool={group.tool} className="shrink-0" />
					<span className="sr-only">, </span>
					<span className="min-w-0 truncate font-medium text-foreground/90">
						{target}
					</span>
					{group.flagged && mode.flags && <AskedAgainPill target={target} />}
					<span className="sr-only">, </span>
					{!choose && (
						<span className="ml-auto shrink-0 pl-2 text-[11px] tabular-nums text-muted-foreground">
							{countText(group.calls)}
						</span>
					)}
				</p>
				{choose && !lone && choice === undefined && (
					<span className="text-[11px] text-muted-foreground">
						{t("crew.approval.batch.mixed", "Mixed")}
					</span>
				)}
				{choose && (
					<ItemDecision
						value={choice}
						onChange={(decision) => choose.onChoose(group.items, decision)}
						label={t(
							"crew.approval.batch.groupAnswer",
							"Answer for {{count}} calls to {{target}}",
							{
								count: group.calls,
								target,
								defaultValue_one: "Answer for {{count}} call to {{target}}",
							},
						)}
						alwaysBlocked={blockText(block)}
						disabled={choose.disabled}
					/>
				)}
			</div>
			{mode.calls && (
				<ul className="mt-1.5 ml-1 space-y-1 border-l border-border/70 pl-3">
					{group.items.map((item) => (
						<BatchItemRow
							key={item.argsHash}
							item={item}
							text={textOf(item)}
							mode={mode}
							decides={!lone}
						/>
					))}
				</ul>
			)}
		</li>
	);
}

/** The groups in one framed list; `capped`, it scrolls inside its frame. */
export function BatchList({
	id,
	listRef,
	groups,
	mode,
	capped,
}: Readonly<{
	id: string;
	listRef: RefObject<HTMLDivElement | null>;
	groups: readonly CallGroup[];
	mode: ListMode;
	capped: boolean;
}>) {
	return (
		<div
			id={id}
			ref={listRef}
			data-slot="approval-batch-frame"
			className={cn(
				"@container rounded-lg border border-border/60 bg-background/60",
				capped && "max-h-[min(26rem,55vh)] overflow-y-auto overscroll-contain",
			)}
		>
			<ul
				data-slot="approval-batch-list"
				className="divide-y divide-border/60 px-3"
			>
				{groups.map((group) => (
					<BatchGroup key={group.key} group={group} mode={mode} />
				))}
			</ul>
		</div>
	);
}

interface SecretActions {
	save: (
		item: ConversationApprovalItem,
		value: string,
		sealed: boolean,
	) => Promise<void>;
	decline: (item: ConversationApprovalItem) => void;
	sealing: BotSecretSealing;
}

/** The batch's secret requests: each saved or declined before the card can be answered. */
function SecretRows({
	items,
	botName,
	answerOf,
	disabled,
	actions,
}: Readonly<{
	items: readonly ConversationApprovalItem[];
	botName: string;
	answerOf: ChoiceOf;
	disabled: boolean;
	actions: SecretActions;
}>) {
	return (
		<div className="space-y-3 rounded-lg border border-border/60 bg-background/60 px-3 py-2.5">
			{items.map((item) => {
				const secret = item.secret;
				if (!secret) return null;
				const answer = answerOf(item);
				return answer ? (
					<p
						key={item.argsHash}
						className="flex flex-wrap items-center gap-1.5 text-xs"
					>
						<ToolChip tool={secret.name} />
						<DecisionPill decision={answer} secret />
					</p>
				) : (
					<SecretRequestForm
						key={item.argsHash}
						compact
						secret={secret}
						botName={botName}
						disabled={disabled}
						sealing={actions.sealing}
						onSave={(value, sealed) => actions.save(item, value, sealed)}
						onDecline={() => actions.decline(item)}
					/>
				);
			})}
		</div>
	);
}

function ShowAll({
	open,
	listId,
	calls,
	groups,
	closedGroups,
	onToggle,
	className,
}: Readonly<{
	open: boolean;
	listId: string;
	calls: number;
	groups: number;
	closedGroups: boolean;
	onToggle: () => void;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const label = open
		? t("crew.approval.batch.showFewer", "Show fewer")
		: closedGroups
			? t("crew.approval.batch.showCalls", "Show calls")
			: groups > CLOSED_GROUPS
				? t(
						"crew.approval.batch.showAllGroups",
						"Show all {{count}} calls in {{groups}} groups",
						{
							count: calls,
							groups,
							defaultValue_one: "Show all {{count}} call in {{groups}} groups",
						},
					)
				: t("crew.approval.batch.showAll", "Show all {{count}} calls", {
						count: calls,
						defaultValue_one: "Show the call",
					});
	return (
		<button
			type="button"
			aria-expanded={open}
			aria-controls={listId}
			onClick={onToggle}
			className={cn(
				"inline-flex items-center gap-1 rounded text-xs font-medium text-foreground/75 outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40",
				className,
			)}
		>
			{label}
			<ChevronDownIcon
				className={cn("size-3 transition-transform", open && "rotate-180")}
				aria-hidden
			/>
		</button>
	);
}

type Mode = "list" | "choose";

/**
 * Below 24rem the footer's start and its secondary button share the first row and the allowing
 * buttons the second, instead of three ragged rows.
 */
const NARROW_FOOTER =
	"@container @max-sm:[&>div:first-child]:basis-auto @max-sm:[&>div:last-child]:contents";
const NARROW_ALLOW = "@max-sm:grow @max-sm:basis-[40%]";

/** Runs one focus move after the next render, when the element it needs exists. */
function useFocusAfterRender() {
	const next = useRef<(() => void) | undefined>(undefined);
	useEffect(() => {
		const move = next.current;
		next.current = undefined;
		move?.();
	});
	return (move: () => void) => {
		next.current = move;
	};
}

/** The checked choice of the list's first radio group, else its first enabled one. */
function focusFirstChoice(list: HTMLElement | null) {
	const checked = list?.querySelector<HTMLElement>(
		'[role="radio"][data-state="checked"]',
	);
	(
		checked ??
		list?.querySelector<HTMLElement>('[role="radio"]:not([disabled])')
	)?.focus();
}

function useEyebrowText(
	answer: ApprovalDecision | undefined,
	closure: "expired" | "cancelled" | undefined,
	tally: Tally,
	uniform: ApprovalDecision | undefined,
	calls: number,
): string {
	const { t } = useTranslation("bots");
	const decisionText = useDecisionText();
	const countText = useCountText();
	if (closure === "expired") return t("crew.approval.expired", "Expired");
	if (closure === "cancelled")
		return t("crew.approval.cancelled", "Task cancelled");
	if (!answer)
		return `${t("crew.approval.label", "Approval request")} · ${countText(calls)}`;
	return uniform
		? t(
				"crew.approval.batch.answeredUniform",
				"{{decision}} · {{count}} calls",
				{
					decision: decisionText(uniform),
					count: calls,
					defaultValue_one: "{{decision}} · {{count}} call",
				},
			)
		: t(
				"crew.approval.batch.answered",
				"{{allowed}} allowed, {{denied}} denied",
				{
					allowed: tally.allowed,
					denied: tally.denied,
				},
			);
}

function markOf(
	waiting: boolean,
	closure: "expired" | "cancelled" | undefined,
	tally: Tally,
): { tone: RequestTone; icon: LucideIcon } {
	if (waiting) return { tone: "warning", icon: ShieldQuestionIcon };
	if (closure === "expired") return { tone: "muted", icon: TimerOffIcon };
	if (closure === "cancelled") return { tone: "muted", icon: CircleSlashIcon };
	return tally.allowed > 0
		? { tone: "good", icon: CheckIcon }
		: { tone: "critical", icon: BanIcon };
}

/** One card for every call of a model round that needs the owner's answer. */
export function ApprovalBatchCard({
	approval,
	botId,
	botName,
	botAppearance,
	readOnly,
}: Readonly<{
	approval: ConversationApproval;
	botId: string;
	botName: string;
	botAppearance?: Partial<BotAppearance>;
	readOnly: boolean;
}>) {
	const { t } = useTranslation("bots");
	const items = approval.items ?? [];
	const state = useApprovalAnswer(approval, botId);
	const upsertSecret = useUpsertBotSecret();
	const sealing = useBotSecretSealing(
		items.some((item) => item.secret) ? botId : null,
	);
	const listId = useId();
	const [mode, setMode] = useState<Mode>("list");
	const [open, setOpen] = useState(false);
	const [choices, setChoices] = useState<Record<string, ApprovalDecision>>({});
	const [secretAnswers, setSecretAnswers] = useState<
		Record<string, ApprovalDecision>
	>({});
	const radiosRef = useRef<HTMLDivElement>(null);
	const chooseRef = useRef<HTMLButtonElement>(null);
	const statusRef = useRef<HTMLOutputElement>(null);
	const focusAfter = useFocusAfterRender();

	const secrets = items.filter((item) => item.secret);
	const calls = callCount(items);
	const groups = useMemo(
		() => groupItems(items.filter((item) => !item.secret)),
		[items],
	);
	const waiting = !state.answer && !state.closure;
	const secretsPending = secrets.some((item) => !secretAnswers[item.argsHash]);
	const busy = readOnly || state.pending || state.elsewhere;
	const choiceOf: ChoiceOf = (item) =>
		item.secret
			? secretAnswers[item.argsHash]
			: (choices[item.argsHash] ?? "once");
	const tally = tallyOf(items, waiting ? choiceOf : state.choiceOf);
	const answered = new Set(items.map(state.choiceOf));
	const uniform =
		state.answer && answered.size === 1 ? [...answered][0] : undefined;
	const { tone, icon } = markOf(waiting, state.closure, tally);
	const eyebrow = useEyebrowText(
		state.answer,
		state.closure,
		tally,
		uniform,
		calls,
	);
	const alwaysAll = items.every((item) => !alwaysBlock(item));

	const send = (answer: BatchAnswer) =>
		state.send(answer.decision, answer.items, () =>
			focusAfter(() => statusRef.current?.focus()),
		);
	const shortcut = (decision: ApprovalDecision) =>
		send(
			decision === "deny"
				? { decision, items: [] }
				: batchAnswer(
						items,
						(item) => (item.secret ? choiceOf(item) : decision),
						decision,
					),
		);
	const choose = (
		chosen: readonly ConversationApprovalItem[],
		decision: ApprovalDecision,
	) =>
		setChoices((current) => ({
			...current,
			...Object.fromEntries(chosen.map((item) => [item.argsHash, decision])),
		}));
	const answerSecret = (
		item: ConversationApprovalItem,
		decision: ApprovalDecision,
	) =>
		setSecretAnswers((current) => ({ ...current, [item.argsHash]: decision }));
	const secretActions: SecretActions = {
		save: async (item, value, sealed) => {
			if (!item.secret) return;
			await upsertSecret.mutateAsync({
				botId,
				name: item.secret.name,
				request: { value, description: item.secret.description },
				sealed,
			});
			answerSecret(item, "once");
		},
		decline: (item) => answerSecret(item, "deny"),
		sealing,
	};

	const few = calls <= FEW_CALLS;
	const listOpen = waiting ? mode === "choose" || open || few : open;
	const listMode: ListMode = {
		calls: listOpen,
		flags: waiting,
		...(waiting && mode === "choose"
			? { choose: { choiceOf, onChoose: choose, disabled: busy } }
			: {}),
		...(state.answer ? { answerOf: state.choiceOf } : {}),
	};
	const shownGroups = listOpen ? groups : groups.slice(0, CLOSED_GROUPS);
	const showBody = waiting || open;
	const capped = listOpen && !few;

	return (
		<RequestCard
			data-slot="approval-card"
			data-batch=""
			aria-label={t(
				"crew.approval.batch.label",
				"Approval request, {{count}} calls",
				{
					count: calls,
					defaultValue_one: "Approval request, {{count}} call",
				},
			)}
			tone={tone}
			live={waiting}
			className="max-w-prose"
		>
			<RequestCardHeader
				dense
				tone={tone}
				live={waiting}
				mark={
					<RequestMark icon={icon} tone={tone}>
						{botAppearance && (
							<BotAvatar
								appearance={botAppearance}
								size={36}
								animated={false}
								decorative
							/>
						)}
					</RequestMark>
				}
				eyebrow={
					<output
						ref={statusRef}
						aria-live="polite"
						tabIndex={-1}
						data-slot={state.answer ? "approval-answer" : undefined}
						className="outline-none"
					>
						{eyebrow}
					</output>
				}
				className={showBody ? undefined : "pb-3.5"}
				title={t(
					"crew.approval.batch.title",
					"{{name}} wants to make {{count}} calls",
					{
						name: botName,
						count: calls,
						defaultValue_one: "{{name}} wants to make {{count}} call",
					},
				)}
				description={
					<>
						<RequesterLine approval={approval} />
						{waiting && items.some((item) => item.afterWebContent) && (
							<WebContentNotice />
						)}
						{state.closure === "expired" && (
							<p className="mt-1 text-xs">
								{t(
									"crew.approval.expiredHint",
									"Nobody answered within 7 days, so {{name}} stopped.",
									{ name: botName },
								)}
							</p>
						)}
						{!waiting && (
							<ShowAll
								open={open}
								listId={listId}
								calls={calls}
								groups={groups.length}
								closedGroups
								onToggle={() => setOpen((value) => !value)}
								className="mt-1.5"
							/>
						)}
					</>
				}
			/>
			{showBody && (
				<RequestCardBody dense className="space-y-2.5 pb-3">
					{secrets.length > 0 && (
						<SecretRows
							items={secrets}
							botName={botName}
							answerOf={waiting ? choiceOf : state.choiceOf}
							disabled={busy}
							actions={secretActions}
						/>
					)}
					{groups.length > 0 && (
						<BatchList
							id={listId}
							listRef={radiosRef}
							groups={shownGroups}
							mode={listMode}
							capped={capped}
						/>
					)}
					{waiting && mode === "list" && !few && (
						<div className="flex justify-center">
							<ShowAll
								open={open}
								listId={listId}
								calls={calls}
								groups={groups.length}
								closedGroups={false}
								onToggle={() => setOpen((value) => !value)}
							/>
						</div>
					)}
				</RequestCardBody>
			)}
			{waiting && (
				<BatchFooter
					view={{
						mode,
						tally,
						alwaysAll,
						busy,
						secretsPending,
						failed: state.failed,
						elsewhere: state.elsewhere,
					}}
					chooseRef={chooseRef}
					actions={{
						choose: () => {
							setMode("choose");
							focusAfter(() => focusFirstChoice(radiosRef.current));
						},
						back: () => {
							setMode("list");
							focusAfter(() => chooseRef.current?.focus());
						},
						shortcut,
						send: () => send(batchAnswer(items, choiceOf)),
					}}
				/>
			)}
		</RequestCard>
	);
}

interface FooterView {
	mode: Mode;
	tally: Tally;
	alwaysAll: boolean;
	busy: boolean;
	secretsPending: boolean;
	failed: boolean;
	elsewhere: boolean;
}

interface FooterActions {
	choose: () => void;
	back: () => void;
	shortcut: (decision: ApprovalDecision) => void;
	send: () => void;
}

function BatchFooter({
	view,
	chooseRef,
	actions,
}: Readonly<{
	view: FooterView;
	chooseRef: RefObject<HTMLButtonElement | null>;
	actions: FooterActions;
}>) {
	const { mode, tally, alwaysAll, busy, secretsPending, failed, elsewhere } =
		view;
	const { t } = useTranslation("bots");
	const blocked = busy || secretsPending;
	const note: ReactNode = failed ? (
		<p role="alert" className="text-xs text-destructive">
			{t("crew.approval.failed", "Couldn't send your answer. Try again.")}
		</p>
	) : elsewhere ? (
		<p className="text-xs text-muted-foreground">
			{t("crew.approval.answeredElsewhere", "Answered on another device")}
		</p>
	) : secretsPending ? (
		<p className="text-xs text-muted-foreground">
			{t("crew.approval.batch.secretFirst", "Save or decline the secret first")}
		</p>
	) : undefined;
	if (mode === "choose") {
		return (
			<RequestCardFooter
				className={cn("px-3.5 py-2.5", NARROW_FOOTER)}
				start={
					note ?? (
						<p
							data-slot="approval-tally"
							aria-live="polite"
							className="text-xs tabular-nums text-muted-foreground"
						>
							{t(
								"crew.approval.batch.tally",
								"{{allowed}} allowed · {{denied}} denied",
								{ allowed: tally.allowed, denied: tally.denied },
							)}
						</p>
					)
				}
			>
				<Button
					type="button"
					size="sm"
					variant="ghost"
					className={requestButtonClass}
					onClick={actions.back}
				>
					{t("crew.approval.batch.back", "Back")}
				</Button>
				<Button
					type="button"
					size="sm"
					className={cn(requestButtonClass, NARROW_ALLOW)}
					disabled={blocked}
					onClick={actions.send}
				>
					<CheckIcon aria-hidden />
					{t("crew.approval.batch.send", "Send answers")}
				</Button>
			</RequestCardFooter>
		);
	}
	return (
		<RequestCardFooter
			className={cn(
				"px-3.5 py-2.5 [&>div:last-child]:max-w-full [&>div:last-child]:flex-wrap [&>div:last-child]:justify-end",
				NARROW_FOOTER,
			)}
			start={
				note ?? (
					<Button
						ref={chooseRef}
						type="button"
						size="sm"
						variant="ghost"
						className={cn(
							requestButtonClass,
							"-ml-2 px-2 text-muted-foreground",
						)}
						disabled={busy}
						onClick={actions.choose}
					>
						{t("crew.approval.batch.choose", "Choose per call")}
					</Button>
				)
			}
		>
			<Button
				type="button"
				size="sm"
				variant="ghost"
				className={requestButtonClass}
				disabled={busy}
				onClick={() => actions.shortcut("deny")}
			>
				<BanIcon aria-hidden />
				{t("crew.approval.batch.denyAll", "Deny all")}
			</Button>
			<Button
				type="button"
				size="sm"
				variant={alwaysAll ? "outline" : "default"}
				className={cn(requestButtonClass, NARROW_ALLOW)}
				disabled={blocked}
				onClick={() => actions.shortcut("once")}
			>
				<CheckIcon aria-hidden />
				{t("crew.approval.batch.allowAllOnce", "Allow all once")}
			</Button>
			{alwaysAll && (
				<Button
					type="button"
					size="sm"
					className={cn(requestButtonClass, NARROW_ALLOW)}
					disabled={blocked}
					onClick={() => actions.shortcut("always")}
				>
					<CheckCheckIcon aria-hidden />
					{t("crew.approval.batch.alwaysAllowAll", "Always allow all")}
				</Button>
			)}
		</RequestCardFooter>
	);
}
