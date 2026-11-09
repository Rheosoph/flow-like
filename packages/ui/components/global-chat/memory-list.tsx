"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	BookmarkIcon,
	BrainIcon,
	CircleAlertIcon,
	Clock3Icon,
	Loader2Icon,
	Trash2Icon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import { cn } from "../../lib/utils";
import {
	type MemoryEntry,
	type MemoryOwner,
	clearGlobalChatMemory,
	deleteGlobalChatMemory,
	listGlobalChatMemories,
} from "../../state/global-chat/global-chat-memory";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { ScrollArea } from "../ui/scroll-area";

type MemoryListState =
	| { status: "loading" }
	| { status: "failed"; message: string }
	| { status: "ready"; entries: MemoryEntry[] };

// Human-friendly "how long ago" for a stored memory's epoch-millis timestamp.
function formatMemoryAge(timestamp: number): string {
	if (!timestamp) return "";
	const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
	if (seconds < 60) return i18next.t("bots:memory.justNow", "just now");
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60)
		return i18next.t("minutesmAgo", "{{minutes}}m ago", { minutes });
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return i18next.t("hourshAgo", "{{hours}}h ago", { hours });
	const days = Math.floor(hours / 24);
	if (days < 30) return i18next.t("daysdAgo", "{{days}}d ago", { days });
	return new Date(timestamp).toLocaleDateString();
}

function MemoryRow({
	entry,
	busy,
	onDelete,
}: Readonly<{
	entry: MemoryEntry;
	busy: boolean;
	onDelete: (id: string) => void;
}>) {
	const { t } = useTranslation("chat");
	const forget = t("forgetThisMemory", "Forget this memory");
	return (
		<li className="group rounded-2xl border border-border/65 bg-card p-4 shadow-xs transition-colors hover:border-emerald-500/30">
			<div className="mb-3 flex items-center gap-2">
				<span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-700 dark:text-emerald-400">
					<BookmarkIcon className="size-3.5" aria-hidden />
				</span>
				<div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
					{entry.role && entry.role !== "observation" && (
						<Badge
							variant="secondary"
							className="max-w-full rounded-md bg-muted/60 px-2 py-0.5 text-[11px] font-medium capitalize wrap-break-word whitespace-normal"
						>
							{entry.role}
						</Badge>
					)}
					{Boolean(entry.timestamp) && (
						<time
							dateTime={new Date(entry.timestamp).toISOString()}
							title={new Date(entry.timestamp).toLocaleString()}
							className="inline-flex items-center gap-1 text-[11px] text-muted-foreground"
						>
							<Clock3Icon className="size-3" aria-hidden />
							{formatMemoryAge(entry.timestamp)}
						</time>
					)}
				</div>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					disabled={busy}
					onClick={() => onDelete(entry.id)}
					aria-label={forget}
					title={forget}
					className="-mr-1 size-8 shrink-0 rounded-lg text-muted-foreground/70 hover:bg-destructive/10 hover:text-destructive focus-visible:text-destructive"
				>
					{busy ? (
						<Loader2Icon
							className="size-3.5 motion-safe:animate-spin"
							aria-hidden
						/>
					) : (
						<Trash2Icon className="size-3.5" aria-hidden />
					)}
				</Button>
			</div>
			<p className="whitespace-pre-wrap wrap-break-word text-sm leading-6 text-foreground">
				{entry.content}
			</p>
		</li>
	);
}

/**
 * The saved memories of one owner (Flux's in a profile, or one bot's), newest first. Reads when
 * it mounts; a deleted entry leaves the list once the backend confirms it.
 */
export function MemoryList({
	owner,
	active = true,
	emptyHint,
	className,
	listClassName,
}: Readonly<{
	owner: MemoryOwner;
	/** False while no embedding model is chosen; saved memories can still be reviewed and deleted. */
	active?: boolean;
	emptyHint?: string;
	className?: string;
	/** Sizes the scrolling list: a max height in a dialog, `flex-1` where it fills a pane. */
	listClassName?: string;
}>) {
	const { t } = useTranslation(["chat", "bots"]);
	const token = useAuth().user?.access_token;
	const [state, setState] = useState<MemoryListState>({ status: "loading" });
	const [busyId, setBusyId] = useState<string | null>(null);
	const [clearing, setClearing] = useState(false);
	const request = useRef(0);

	const load = useCallback(async () => {
		const id = ++request.current;
		setState({ status: "loading" });
		try {
			const entries = await listGlobalChatMemories(owner, token);
			if (id === request.current) setState({ status: "ready", entries });
		} catch (error) {
			if (id === request.current)
				setState({ status: "failed", message: getErrorMessage(error) });
		}
	}, [owner, token]);

	useEffect(() => {
		void load();
		return () => {
			request.current++;
		};
	}, [load]);

	const remove = useCallback(
		async (id: string) => {
			setBusyId(id);
			try {
				await deleteGlobalChatMemory(owner, id, token);
				setState((current) =>
					current.status === "ready"
						? {
								...current,
								entries: current.entries.filter((entry) => entry.id !== id),
							}
						: current,
				);
			} catch {
				toast.error(t("bots:memory.deleteFailed", "Couldn't delete memory"));
			} finally {
				setBusyId(null);
			}
		},
		[owner, token, t],
	);

	const clearAll = useCallback(async () => {
		setClearing(true);
		try {
			await clearGlobalChatMemory(owner, token);
			request.current++;
			setState({ status: "ready", entries: [] });
			toast.success(t("bots:memory.cleared", "Cleared all memories"));
		} catch {
			toast.error(t("bots:memory.clearFailed", "Couldn't clear memories"));
		} finally {
			setClearing(false);
		}
	}, [owner, token, t]);

	const entries = state.status === "ready" ? state.entries : [];

	return (
		<div className={cn("flex min-h-0 min-w-0 flex-col gap-4", className)}>
			{!active && (
				<p className="rounded-xl border border-border/60 bg-muted/30 px-3.5 py-3 text-xs leading-5 text-muted-foreground">
					{t(
						"bots:memory.off",
						"Memory is off. Choose an embedding model in bot settings to recall and save memories. You can still review and delete saved memories here.",
					)}
				</p>
			)}
			{state.status === "ready" && entries.length > 0 && (
				<div className="flex shrink-0 items-center gap-2.5 px-1">
					<span className="flex size-8 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-700 dark:text-emerald-400">
						<BrainIcon className="size-4" aria-hidden />
					</span>
					<p className="text-sm font-medium tracking-tight">
						{t("bots:memory.count", "{{count}} memories", {
							count: entries.length,
							defaultValue_one: "{{count}} memory",
						})}
					</p>
				</div>
			)}
			<ScrollArea
				className={cn("min-h-0 min-w-0 pr-3", listClassName)}
				viewportClassName="max-h-[inherit] [&>div]:block! [&>div]:w-full [&>div]:min-w-0"
			>
				{state.status === "loading" && (
					<div className="space-y-3 py-1">
						<output className="flex items-center justify-center gap-2 py-5 text-xs text-muted-foreground">
							<Loader2Icon
								className="size-4 motion-safe:animate-spin"
								aria-hidden
							/>
							{t("loading", "Loading…")}
						</output>
						{[0, 1, 2].map((row) => (
							<div
								key={row}
								aria-hidden
								className="space-y-3 rounded-2xl border border-border/50 p-4 motion-safe:animate-pulse"
							>
								<div className="h-5 w-24 rounded-md bg-muted" />
								<div className="h-3 w-full rounded bg-muted/70" />
								<div className="h-3 w-2/3 rounded bg-muted/70" />
							</div>
						))}
					</div>
				)}
				{state.status === "failed" && (
					<div
						role="alert"
						className="flex flex-col items-center gap-3 rounded-2xl border border-border/60 bg-muted/20 px-5 py-10 text-center text-sm text-muted-foreground"
					>
						<span className="flex size-11 items-center justify-center rounded-2xl bg-muted">
							<CircleAlertIcon className="size-5" aria-hidden />
						</span>
						<p className="font-medium text-foreground">
							{t("bots:memory.loadFailed", "Couldn't load memories")}
						</p>
						<p className="max-w-80 text-xs leading-5 wrap-break-word">
							{state.message}
						</p>
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => void load()}
							className="mt-1 rounded-lg"
						>
							{t("bots:crew.retry", "Try again")}
						</Button>
					</div>
				)}
				{state.status === "ready" && entries.length === 0 && (
					<div className="flex flex-col items-center rounded-2xl border border-dashed border-border/80 bg-muted/15 px-6 py-12 text-center">
						<div className="mb-5 rounded-full bg-emerald-500/5 p-3">
							<span className="flex size-14 items-center justify-center rounded-2xl border border-emerald-500/15 bg-background shadow-xs">
								<BrainIcon
									className="size-6 text-emerald-700/80 dark:text-emerald-400/80"
									aria-hidden
								/>
							</span>
						</div>
						<p className="text-sm font-medium tracking-tight">
							{t("noMemoriesSavedYet", "No memories saved yet.")}
						</p>
						<p className="mt-2 max-w-64 text-xs leading-5 text-muted-foreground">
							{emptyHint ??
								t(
									"theAssistantStoresSalientFactsAsYouChat",
									"The assistant stores salient facts as you chat.",
								)}
						</p>
					</div>
				)}
				{entries.length > 0 && (
					<ul className="flex flex-col gap-3 py-1">
						{entries.map((entry) => (
							<MemoryRow
								key={entry.id}
								entry={entry}
								busy={busyId === entry.id}
								onDelete={remove}
							/>
						))}
					</ul>
				)}
			</ScrollArea>
			{entries.length > 0 && (
				<div className="flex shrink-0 items-center justify-end border-t border-border/60 pt-3">
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={clearing}
						onClick={clearAll}
						className="gap-1.5 rounded-lg text-xs text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
					>
						{clearing ? (
							<Loader2Icon className="size-3.5 animate-spin" aria-hidden />
						) : (
							<Trash2Icon className="size-3.5" aria-hidden />
						)}
						{t("clearAll", "Clear all")}
					</Button>
				</div>
			)}
		</div>
	);
}
