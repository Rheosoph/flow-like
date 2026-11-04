"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2Icon, Trash2Icon } from "lucide-react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import {
	type BotFact,
	forgetBotFact,
	listBotFacts,
} from "../../state/global-chat/bot-memory-facts";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";

export function botFactsKey(botId: string) {
	return ["bot-memory", "facts", botId] as const;
}

function FactRow({
	fact,
	busy,
	onForget,
	onJump,
}: Readonly<{
	fact: BotFact;
	busy: boolean;
	onForget: (id: string) => void;
	onJump: (seq: number) => void;
}>) {
	const { t } = useTranslation("bots");
	const forget = t("memory.forget", "Forget this fact");
	return (
		<li className="flex items-start gap-2 rounded-lg border border-border/50 bg-muted/30 p-2.5">
			<div className="min-w-0 flex-1">
				<p className="whitespace-pre-wrap wrap-break-word text-sm">
					{fact.text}
				</p>
				<div className="mt-1 flex flex-wrap items-center gap-1.5">
					<Badge
						variant="secondary"
						className="h-4 px-1.5 text-[10px] font-normal"
					>
						{fact.kind}
					</Badge>
					{fact.evidence.map((evidence) => (
						<button
							key={`${evidence.message_id}-${evidence.seq}`}
							type="button"
							onClick={() => onJump(evidence.seq)}
							title={evidence.quote}
							className="rounded px-1 text-[11px] text-primary underline-offset-2 hover:underline focus-visible:underline"
						>
							#{evidence.seq}
						</button>
					))}
				</div>
				{fact.evidence[0]?.quote && (
					<blockquote className="mt-1.5 border-l-2 pl-2 text-xs text-muted-foreground italic">
						{fact.evidence[0].quote}
					</blockquote>
				)}
			</div>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				disabled={busy}
				onClick={() => onForget(fact.id)}
				aria-label={forget}
				title={forget}
				className="size-7 shrink-0 text-muted-foreground hover:text-destructive"
			>
				{busy ? (
					<Loader2Icon className="size-3.5 animate-spin" aria-hidden />
				) : (
					<Trash2Icon className="size-3.5" aria-hidden />
				)}
			</Button>
		</li>
	);
}

/**
 * The facts a bot remembers, each with the quote it came from and a link to that place in the
 * chat. Forgetting deletes the fact itself, not the chat that said it.
 */
export function BotFactsList({
	botId,
	appId,
	name,
	memoryOff = false,
	onJump,
}: Readonly<{
	botId: string;
	appId: string;
	name: string;
	/** Memory is off for this bot, so an empty list isn't "as you chat". */
	memoryOff?: boolean;
	onJump: (seq: number) => void;
}>) {
	const { t } = useTranslation("bots");
	const token = useAuth().user?.access_token;
	const queryClient = useQueryClient();
	const owner = { botId, appId, token };
	const facts = useQuery({
		queryKey: botFactsKey(botId),
		queryFn: () => listBotFacts(owner),
		retry: false,
	});
	const forget = useMutation({
		mutationFn: (id: string) => forgetBotFact(owner, id),
		onSuccess: (_, id) =>
			queryClient.setQueryData<BotFact[]>(botFactsKey(botId), (current) =>
				current?.filter((fact) => fact.id !== id),
			),
		onError: (error) => toast.error(getErrorMessage(error)),
	});

	if (facts.isPending) {
		return (
			<p className="px-4 py-3 text-xs text-muted-foreground">
				{t("crew.dock.loading", "Loading…")}
			</p>
		);
	}
	if (facts.isError) {
		return (
			<div className="flex flex-col items-start gap-2 px-4 py-3">
				<p className="text-xs text-muted-foreground">
					{t("memory.factsFailed", "Couldn't load {{name}}'s facts.", { name })}{" "}
					{getErrorMessage(facts.error)}
				</p>
				<Button
					type="button"
					size="sm"
					variant="outline"
					onClick={() => void facts.refetch()}
				>
					{t("crew.retry", "Try again")}
				</Button>
			</div>
		);
	}
	if (facts.data.length === 0) {
		return (
			<p className="px-4 py-3 text-xs text-muted-foreground">
				{memoryOff
					? t("memory.factsEmptyOff", "Nothing is saved while memory is off.")
					: t(
							"memory.factsEmpty",
							"{{name}} writes down facts, preferences and decisions here as you chat.",
							{ name },
						)}
			</p>
		);
	}
	return (
		<ul className="flex flex-1 flex-col gap-2 overflow-y-auto px-4 pt-3 pb-4">
			{facts.data.map((fact) => (
				<FactRow
					key={fact.id}
					fact={fact}
					busy={forget.isPending && forget.variables === fact.id}
					onForget={(id) => forget.mutate(id)}
					onJump={onJump}
				/>
			))}
		</ul>
	);
}
