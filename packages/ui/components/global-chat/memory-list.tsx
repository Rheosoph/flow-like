"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { BrainIcon, Loader2Icon, Trash2Icon } from "lucide-react";
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
		<li className="group flex items-start gap-2 rounded-lg border border-border/50 bg-muted/30 p-2.5">
			<div className="min-w-0 flex-1">
				<p className="whitespace-pre-wrap wrap-break-word text-sm text-foreground">
					{entry.content}
				</p>
				<div className="mt-1.5 flex items-center gap-1.5">
					{entry.role && entry.role !== "observation" && (
						<Badge
							variant="secondary"
							className="h-4 px-1.5 text-[10px] font-normal"
						>
							{entry.role}
						</Badge>
					)}
					<span className="text-[11px] text-muted-foreground">
						{formatMemoryAge(entry.timestamp)}
					</span>
				</div>
			</div>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				disabled={busy}
				onClick={() => onDelete(entry.id)}
				aria-label={forget}
				title={forget}
				className="size-7 shrink-0 text-muted-foreground hover:text-destructive"
			>
				{busy ? (
					<Loader2Icon className="size-3.5 animate-spin" />
				) : (
					<Trash2Icon className="size-3.5" />
				)}
			</Button>
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
		<div className={cn("flex min-h-0 flex-col gap-3", className)}>
			{!active && (
				<p className="rounded-md border border-border/50 bg-muted/40 px-2.5 py-2 text-xs text-muted-foreground">
					{t(
						"bots:memory.off",
						"Memory is off. Choose an embedding model in bot settings to recall and save memories. You can still review and delete saved memories here.",
					)}
				</p>
			)}
			<ScrollArea
				className={cn("min-h-0 pr-3", listClassName)}
				viewportClassName="max-h-[inherit] [&>div]:block! [&>div]:w-full [&>div]:min-w-0"
			>
				{state.status === "loading" && (
					<output className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
						<Loader2Icon className="size-4 animate-spin" aria-hidden />
						{t("loading", "Loading…")}
					</output>
				)}
				{state.status === "failed" && (
					<div
						role="alert"
						className="flex flex-col items-center gap-2 py-10 text-center text-sm text-muted-foreground"
					>
						<p>{t("bots:memory.loadFailed", "Couldn't load memories")}</p>
						<p className="max-w-80 text-xs wrap-break-word">{state.message}</p>
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => void load()}
						>
							{t("bots:crew.retry", "Try again")}
						</Button>
					</div>
				)}
				{state.status === "ready" && entries.length === 0 && (
					<div className="flex flex-col items-center gap-1 py-10 text-center text-sm text-muted-foreground">
						<BrainIcon className="size-6 opacity-40" aria-hidden />
						<p>{t("noMemoriesSavedYet", "No memories saved yet.")}</p>
						<p className="text-xs">
							{emptyHint ??
								t(
									"theAssistantStoresSalientFactsAsYouChat",
									"The assistant stores salient facts as you chat.",
								)}
						</p>
					</div>
				)}
				{entries.length > 0 && (
					<ul className="flex flex-col gap-2 py-1">
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
				<div className="flex shrink-0 items-center justify-between gap-2">
					<span className="text-xs text-muted-foreground">
						{t("bots:memory.count", "{{count}} memories", {
							count: entries.length,
							defaultValue_one: "{{count}} memory",
						})}
					</span>
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={clearing}
						onClick={clearAll}
						className="gap-1.5 text-destructive hover:text-destructive"
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
