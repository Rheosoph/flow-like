"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleAlertIcon,
	CircleCheckIcon,
	Loader2Icon,
	TriangleAlertIcon,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { useBotLabels } from "../../hooks/use-bots";
import { useInvoke } from "../../hooks/use-invoke";
import { ApiResponseError } from "../../lib/api-error";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type {
	BotMoveStep,
	IBot,
	IBotMoveIssue,
	IBotMoveReport,
	ILocalBotHost,
} from "../../state/backend-state/bot-state";
import { BotAvatar } from "../bots/bot-avatar";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	useBotErrorMessage,
	useServerLimitFact,
} from "./editor/bot-editor-copy";
import { BOT_SERVER_LIMIT_REACHED } from "./editor/bot-limits";
import {
	BOT_LOOK_NOT_OWNED,
	BOT_MODEL_UNAVAILABLE,
	BOT_SECRET_UNREADABLE,
	LOCAL_CHANGED,
	type MoveFix,
	SECRET_KEY,
	moveFixes,
	staysOnThisComputer,
} from "./move-checks";
import {
	type BotMoveReview,
	useMoveFix,
	useMoveReviews,
	useMoveRun,
} from "./use-local-bot-moves";

export interface MoveLocalBotsDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Every bot that lives only on this computer. */
	bots: readonly IBot[];
	/** Reviews just this bot, as its intro asks. */
	botId?: string | null;
	onEditBot: (botId: string) => void;
	/** The first bot that moved, once the run is over. */
	onMoved: (bot: IBot) => void;
}

/** Library app names for issues about apps; an app the library doesn't know shows its id. */
function useAppName() {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const names = useMemo(
		() =>
			new Map(
				(apps.data ?? []).map(([app, metadata]) => [app.id, metadata?.name]),
			),
		[apps.data],
	);
	return useCallback(
		(appId: string) => names.get(appId)?.trim() || appId,
		[names],
	);
}

type IssueText = (ref: string, issue: IBotMoveIssue, bot: IBot) => string;

/** Words for the issue codes the review knows; `null` for any other code. */
function useKnownIssueText() {
	const { t } = useTranslation("bots");
	const appName = useAppName();
	const limitFact = useServerLimitFact();
	const texts = useMemo(
		() =>
			new Map<string, IssueText>([
				[
					"BOT_LIMIT_REACHED",
					() =>
						t(
							"move.issues.limitReached",
							"Your account already holds as many bots as it can.",
						),
				],
				[
					BOT_SERVER_LIMIT_REACHED,
					(_, issue) =>
						t(
							"move.issues.serverLimitReached",
							"{{fact}}. Delete one of your bots on the server or upgrade your plan to move it.",
							{ fact: limitFact(issue.message) },
						),
				],
				[
					"BOT_INVALID",
					(_, issue) =>
						t(
							"move.issues.invalid",
							"Some of its settings don't fit a bot in an account: {{message}}",
							{ message: issue.message },
						),
				],
				[
					"BOT_CAPABILITY_LOCATION",
					() =>
						t(
							"move.issues.capabilityLocation",
							"One of its abilities doesn't fit where it runs.",
						),
				],
				[
					"BOT_PROFILE_UNAVAILABLE",
					() =>
						t(
							"move.issues.profileUnavailable",
							"The profile its apps come from isn't in your account.",
						),
				],
				[
					"BOT_DEVICE_UNAVAILABLE",
					() =>
						t(
							"move.issues.deviceUnavailable",
							"The device it runs on isn't in your account.",
						),
				],
				[
					"BOT_APP_UNAVAILABLE",
					(ref) => {
						const app = appName(ref);
						return t(
							"move.issues.appUnavailable",
							"{{app}} isn't available to your account.",
							{ app },
						);
					},
				],
				[
					"BOT_APP_LIMIT_REACHED",
					() =>
						t(
							"move.issues.appLimit",
							"It uses more apps than a bot in an account may.",
						),
				],
				[
					BOT_MODEL_UNAVAILABLE,
					(ref, _, bot) =>
						ref === bot.model.primary
							? t(
									"move.issues.modelUnavailable",
									"Its model {{model}} isn't available to your account.",
									{ model: ref },
								)
							: t(
									"move.issues.fallbackUnavailable",
									"Its fallback model {{model}} isn't available to your account.",
									{ model: ref },
								),
				],
				[
					BOT_LOOK_NOT_OWNED,
					(_, issue) =>
						t(
							"move.issues.lookNotOwned",
							"Its look uses items your account doesn't own: {{items}}.",
							{ items: issue.message },
						),
				],
				[
					BOT_SECRET_UNREADABLE,
					(ref) =>
						t(
							"move.issues.secretUnreadable",
							"Its secret {{name}} can't be read on this computer anymore. Remove it here and add it again once the bot has moved.",
							{ name: ref },
						),
				],
				[
					LOCAL_CHANGED,
					() =>
						t(
							"move.issues.localChanged",
							"It kept changing on this computer while it moved. Try again in a moment.",
						),
				],
				[
					SECRET_KEY,
					() =>
						t(
							"move.issues.secretKey",
							"This computer's keychain is locked or refused access to the bot's secrets. Unlock it and try again.",
						),
				],
			]),
		[appName, limitFact, t],
	);
	return useCallback(
		(issue: IBotMoveIssue, bot: IBot): string | null =>
			texts.get(issue.code)?.(issue.ref_id ?? "", issue, bot) ?? null,
		[texts],
	);
}

/** An issue of the review, and why a move failed: known codes in words, else what was said. */
function useMoveTexts() {
	const { t } = useTranslation("bots");
	const known = useKnownIssueText();
	const errorMessage = useBotErrorMessage();
	return useMemo(() => {
		const issueText = (issue: IBotMoveIssue, bot: IBot) =>
			known(issue, bot) ??
			(issue.message.trim() ||
				t(
					"move.issues.unknown",
					"Something about it doesn't fit your account.",
				));
		const failureText = (cause: unknown, bot: IBot) => {
			const message = cause instanceof Error ? cause.message.trim() : "";
			const failure = (cause ?? {}) as {
				issue?: IBotMoveIssue;
				code?: unknown;
			};
			const issue =
				failure.issue ??
				(typeof failure.code === "string"
					? { part: "bot" as const, code: failure.code, message, ref_id: null }
					: undefined);
			const said = issue && known(issue, bot);
			if (said) return said;
			const fallback = t("move.result.unknownError", "Something went wrong.");
			if (cause instanceof ApiResponseError)
				return errorMessage(cause, fallback);
			if (message) return message;
			return typeof cause === "string" && cause.trim() ? cause : fallback;
		};
		return { issueText, failureText };
	}, [errorMessage, known, t]);
}

function useFixLabel() {
	const { t } = useTranslation("bots");
	return useCallback(
		(fix: MoveFix): string => {
			switch (fix.kind) {
				case "detach_app":
					return t("move.fix.removeApp", "Remove from bot");
				case "use_profile":
					return t("move.fix.useProfile", "Use current profile");
				case "drop_profile_apps":
					return t("move.fix.dropProfileApps", "Switch off profile apps");
				case "automatic_model":
					return t("move.fix.automaticModel", "Let the host choose");
				case "drop_fallback":
					return t("move.fix.dropFallback", "Remove fallback");
				case "remove_secret":
					return t("move.fix.removeSecret", "Remove secret");
				case "open_settings":
					return t("move.fix.openSettings", "Open settings");
			}
		},
		[t],
	);
}

function MoveParts({ report }: Readonly<{ report: IBotMoveReport }>) {
	const { t } = useTranslation("bots");
	const parts: string[] = [];
	if (report.apps > 0) {
		parts.push(
			t("move.parts.apps", "{{count}} apps", {
				count: report.apps,
				defaultValue_one: "{{count}} app",
			}),
		);
	}
	if (report.secrets > 0) {
		parts.push(
			t(
				"move.parts.secrets",
				"{{count}} secrets, stored encrypted in your account",
				{
					count: report.secrets,
					defaultValue_one:
						"{{count}} secret, stored encrypted in your account",
				},
			),
		);
	}
	if (report.conversations > 0) {
		parts.push(
			t("move.parts.chats", "{{count}} chats with {{messages}}", {
				count: report.conversations,
				messages: t("move.parts.messages", "{{count}} messages", {
					count: report.messages,
					defaultValue_one: "{{count}} message",
				}),
				defaultValue_one: "{{count}} chat with {{messages}}",
			}),
		);
	}
	if (report.waiting_conversations > 0) {
		parts.push(
			t(
				"move.parts.waiting",
				"{{count}} chats move once the other bots in them do",
				{
					count: report.waiting_conversations,
					defaultValue_one: "{{count}} chat moves once the other bots in it do",
				},
			),
		);
	}
	if (report.skipped_messages > 0) {
		parts.push(
			t(
				"move.parts.skipped",
				"{{count}} messages from bots that left these chats are left out",
				{
					count: report.skipped_messages,
					defaultValue_one:
						"{{count}} message from a bot that left its chat is left out",
				},
			),
		);
	}
	return (
		<ul
			aria-label={t("move.parts.label", "Moves along")}
			className="flex flex-col gap-0.5 text-xs text-muted-foreground"
		>
			{parts.length === 0 ? (
				<li>{t("move.parts.nothing", "Just the bot itself")}</li>
			) : (
				parts.map((part) => <li key={part}>{part}</li>)
			)}
		</ul>
	);
}

function StatusPill({
	icon,
	tone,
	children,
}: Readonly<{
	icon: ReactNode;
	tone: "muted" | "warning" | "done";
	children: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-2 text-xs font-medium",
				tone === "muted" && "bg-muted text-muted-foreground",
				tone === "warning" && "bg-warning-bg text-warning",
				tone === "done" && "bg-primary/10 text-primary",
			)}
		>
			{icon}
			{children}
		</span>
	);
}

function useMoveStepText() {
	const { t } = useTranslation("bots");
	return useCallback(
		(step: BotMoveStep): string =>
			({
				check: t("move.steps.check", "Checking it against your account…"),
				import_bot: t("move.steps.import_bot", "Creating it in your account…"),
				update_bot: t("move.steps.update_bot", "Copying its settings…"),
				apps: t("move.steps.apps", "Attaching its apps…"),
				secrets: t("move.steps.secrets", "Moving its secrets…"),
				workspace: t("move.steps.workspace", "Copying its files…"),
				conversations: t("move.steps.conversations", "Moving its chats…"),
				segments: t("move.steps.segments", "Moving its chat summaries…"),
				forget_conversations: t(
					"move.steps.forget_conversations",
					"Clearing the chats here…",
				),
				verify: t("move.steps.verify", "Checking nothing changed meanwhile…"),
				forget_workspace: t(
					"move.steps.forget_workspace",
					"Removing its files here…",
				),
				rebind: t("move.steps.rebind", "Switching it to your account…"),
				forget_bot: t(
					"move.steps.forget_bot",
					"Removing it from this computer…",
				),
				done: t("move.steps.done", "Finishing up…"),
			})[step],
		[t],
	);
}

function MoveStatus({ review }: Readonly<{ review: BotMoveReview }>) {
	const { t } = useTranslation("bots");
	const spinner = <Loader2Icon className="size-3 animate-spin" aria-hidden />;
	const status = review.outcome?.status;
	if (status === "moving")
		return (
			<StatusPill icon={spinner} tone="muted">
				{t("move.status.moving", "Moving…")}
			</StatusPill>
		);
	if (status === "moved")
		return (
			<StatusPill
				icon={<CircleCheckIcon className="size-3" aria-hidden />}
				tone="done"
			>
				{t("move.status.moved", "Moved")}
			</StatusPill>
		);
	if (review.checking)
		return (
			<StatusPill icon={spinner} tone="muted">
				{t("move.status.checking", "Checking…")}
			</StatusPill>
		);
	if (review.checkFailed || review.issues.length > 0)
		return (
			<StatusPill
				icon={<TriangleAlertIcon className="size-3" aria-hidden />}
				tone="warning"
			>
				{review.checkFailed
					? t("move.status.notChecked", "Not checked")
					: t("move.status.issues", "{{count}} issues", {
							count: review.issues.length,
							defaultValue_one: "{{count}} issue",
						})}
			</StatusPill>
		);
	return (
		<StatusPill icon={null} tone="muted">
			{t("move.status.ready", "Ready")}
		</StatusPill>
	);
}

/** What every row needs to describe issues and act on them. */
interface MoveRowContext {
	issueText: (issue: IBotMoveIssue, bot: IBot) => string;
	failureText: (cause: unknown, bot: IBot) => string;
	fixLabel: (fix: MoveFix) => string;
	profileId: string | null;
	running: boolean;
	steps: Readonly<Record<string, BotMoveStep>>;
	onFix: (bot: IBot, fix: MoveFix) => void;
}

function MoveIssueLine({
	issue,
	review,
	busy,
	context,
}: Readonly<{
	issue: IBotMoveIssue;
	review: BotMoveReview;
	busy: boolean;
	context: MoveRowContext;
}>) {
	const { t } = useTranslation("bots");
	const { issueText, fixLabel, profileId, onFix } = context;
	const fixes = moveFixes(issue, review.bot);
	return (
		<li className="flex flex-col gap-2 rounded-lg bg-muted/60 px-3 py-2 text-xs leading-relaxed">
			<span className="flex gap-2">
				<TriangleAlertIcon
					className="mt-0.5 size-3.5 shrink-0 text-warning"
					aria-hidden
				/>
				<span>
					{issueText(issue, review.bot)}
					{staysOnThisComputer(issue) &&
						` ${t("move.staysHere", "It stays on this computer and keeps working.")}`}
				</span>
			</span>
			{fixes.length > 0 && (
				<span className="flex flex-wrap gap-1.5 pl-5.5">
					{fixes.map((fix) => (
						<Button
							key={fix.kind}
							type="button"
							variant="outline"
							size="sm"
							className="h-7 rounded-md px-2.5 text-xs"
							disabled={busy || (fix.kind === "use_profile" && !profileId)}
							onClick={() => onFix(review.bot, fix)}
						>
							{fixLabel(fix)}
						</Button>
					))}
				</span>
			)}
		</li>
	);
}

function MoveBotRow({
	review,
	fixing,
	context,
}: Readonly<{
	review: BotMoveReview;
	fixing: boolean;
	context: MoveRowContext;
}>) {
	const { t } = useTranslation("bots");
	const botLabels = useBotLabels();
	const { bot, outcome, report } = review;
	const name = botLabels(bot).name || t("crew.unknownMember", "Someone");
	const settled = outcome?.status === "moving" || outcome?.status === "moved";
	const stepText = useMoveStepText();
	const step =
		outcome?.status === "moving" ? context.steps[review.id] : undefined;
	return (
		<li className="flex flex-col gap-2.5 rounded-xl border bg-card p-3.5">
			<div className="flex items-start gap-3">
				<span className="flex size-10 shrink-0 items-end justify-center overflow-hidden rounded-[10px] bg-muted">
					<BotAvatar
						appearance={bot.appearance}
						size={40}
						animated={false}
						decorative
					/>
				</span>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<span className="truncate text-sm font-semibold">{name}</span>
					{report && <MoveParts report={report} />}
				</div>
				<MoveStatus review={review} />
			</div>
			{step && (
				<output className="text-xs text-muted-foreground">
					{stepText(step)}
				</output>
			)}
			{outcome?.status === "moved" && outcome.warnings > 0 && (
				<p className="flex items-center gap-2 text-xs text-muted-foreground">
					<TriangleAlertIcon
						className="size-3.5 shrink-0 text-warning"
						aria-hidden
					/>
					{t("move.result.warnings", "{{count}} file links couldn't move", {
						count: outcome.warnings,
						defaultValue_one: "{{count}} file link couldn't move",
					})}
				</p>
			)}
			{review.checkFailed && !settled && (
				<p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
					<CircleAlertIcon className="size-3.5 shrink-0" aria-hidden />
					{t("move.checkFailed", "Couldn't check this bot.")}
					<Button
						type="button"
						variant="link"
						size="sm"
						className="h-auto p-0 text-xs"
						onClick={review.recheck}
					>
						{t("move.recheck", "Check again")}
					</Button>
				</p>
			)}
			{!settled && review.issues.length > 0 && (
				<ul className="flex flex-col gap-1.5">
					{review.issues.map((issue) => (
						<MoveIssueLine
							key={`${issue.part}:${issue.code}:${issue.ref_id ?? ""}`}
							issue={issue}
							review={review}
							busy={fixing || context.running}
							context={context}
						/>
					))}
				</ul>
			)}
			{outcome?.status === "failed" && (
				<p role="alert" className="text-xs text-destructive">
					{t(
						"move.result.failed",
						"Not moved, so it stays on this computer. {{error}}",
						{ error: context.failureText(outcome.cause, bot) },
					)}
				</p>
			)}
		</li>
	);
}

function useAccountName(): string | undefined {
	const auth = useAuth();
	const profile = auth.user?.profile;
	return profile?.email ?? profile?.name ?? undefined;
}

function MoveSession({
	host,
	bots,
	botId,
	onEditBot,
	onMoved,
	onClose,
}: Readonly<{
	host: ILocalBotHost;
	bots: readonly IBot[];
	botId?: string | null;
	onEditBot: (botId: string) => void;
	onMoved: (bot: IBot) => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const account = useAccountName();
	const listed = useMemo(
		() => (botId ? bots.filter((bot) => bot.id === botId) : bots),
		[botId, bots],
	);
	const run = useMoveRun(host, onMoved);
	const reviews = useMoveReviews(host, listed, run.outcomes);
	const { apply, fixing, profileId } = useMoveFix(onEditBot);
	const { issueText, failureText } = useMoveTexts();
	const fixLabel = useFixLabel();
	const ready = reviews.filter((review) => review.ready);
	const running = run.progress !== null;
	const context = useMemo<MoveRowContext>(
		() => ({
			issueText,
			failureText,
			fixLabel,
			profileId,
			running,
			steps: run.steps,
			onFix: (bot, chosen) => void apply(bot, chosen),
		}),
		[apply, failureText, fixLabel, issueText, profileId, running, run.steps],
	);

	return (
		<>
			{account && (
				<p className="text-xs text-muted-foreground">
					{t("move.account", "Signed in as {{account}}", { account })}
				</p>
			)}
			<div className="-mx-1 min-h-0 flex-1 overflow-y-auto px-1">
				{reviews.length === 0 ? (
					<p className="rounded-xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
						{t("move.empty", "No bots live only on this computer.")}
					</p>
				) : (
					<ul className="flex flex-col gap-2.5">
						{reviews.map((review) => (
							<MoveBotRow
								key={review.id}
								review={review}
								fixing={fixing === review.id}
								context={context}
							/>
						))}
					</ul>
				)}
			</div>
			<DialogFooter className="items-center gap-2 sm:justify-between">
				<output className="text-xs text-muted-foreground">
					{run.progress &&
						t("move.progress", "Moving {{current}} of {{total}}…", {
							current: Math.min(run.progress.done + 1, run.progress.total),
							total: run.progress.total,
						})}
				</output>
				<span className="flex gap-2">
					<Button type="button" variant="outline" onClick={onClose}>
						{t("move.close", "Close")}
					</Button>
					<Button
						type="button"
						disabled={running || ready.length === 0}
						onClick={() => void run.move(ready.map((review) => review.bot))}
					>
						{running && (
							<Loader2Icon className="size-4 animate-spin" aria-hidden />
						)}
						{t("move.action", "Move {{count}} bots", {
							count: ready.length,
							defaultValue_one: "Move {{count}} bot",
						})}
					</Button>
				</span>
			</DialogFooter>
		</>
	);
}

/**
 * Reviews the bots that live only on this computer against the signed-in account, offers fixes
 * for what doesn't fit, and moves the ones that do, one at a time.
 */
export function MoveLocalBotsDialog({
	open,
	onOpenChange,
	bots,
	botId,
	onEditBot,
	onMoved,
}: Readonly<MoveLocalBotsDialogProps>) {
	const { t } = useTranslation("bots");
	const host = useBackend().botState?.local;
	const close = useCallback(() => onOpenChange(false), [onOpenChange]);
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85dvh] flex-col sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>
						{t("move.title", "Move bots to your account")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"move.description",
							"Bots you made while signed out live only on this computer. Moving takes their apps, secrets and chats into your account and then removes them here. Bots with open issues stay here and keep working.",
						)}
					</DialogDescription>
				</DialogHeader>
				{open && host && (
					<MoveSession
						host={host}
						bots={bots}
						botId={botId}
						onEditBot={onEditBot}
						onMoved={onMoved}
						onClose={close}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
