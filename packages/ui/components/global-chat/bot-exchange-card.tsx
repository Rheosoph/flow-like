"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRightLeftIcon,
	ClockIcon,
	ExternalLinkIcon,
	ShieldQuestionIcon,
	XCircleIcon,
} from "lucide-react";
import {
	RequestCard,
	RequestCardBody,
	RequestCardHeader,
	RequestMark,
	type RequestTone,
} from "../interfaces/chat-default/request-card";
import type { BotExchangeEntry, BotExchangeView } from "./bot-exchange";

const ICONS = {
	answered: ArrowRightLeftIcon,
	pending: ClockIcon,
	waiting: ShieldQuestionIcon,
	denied: XCircleIcon,
	error: XCircleIcon,
} as const;

const TONES: Record<BotExchangeView["state"], RequestTone> = {
	answered: "muted",
	pending: "primary",
	waiting: "warning",
	denied: "critical",
	error: "critical",
};

function ExchangeLink({
	conversationId,
	onOpen,
}: Readonly<{
	conversationId: string;
	onOpen: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<button
			type="button"
			onClick={() => onOpen(conversationId)}
			className="inline-flex items-center gap-1.5 rounded text-xs font-medium text-primary underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-primary/40"
		>
			<ExternalLinkIcon className="size-3" aria-hidden />
			{t("exchange.open", "Open the full exchange")}
		</button>
	);
}

/** "Scout asked Ledger … · Ledger answered …" with the state of an exchange between two bots. */
export function BotExchangeCard({
	exchange,
	onOpen,
}: Readonly<{
	exchange: BotExchangeView;
	onOpen?: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const Icon = ICONS[exchange.state];
	const vars = {
		caller: exchange.callerName || t("exchange.thisBot", "This bot"),
		callee: exchange.calleeName,
	};
	const headline = {
		answered: t("exchange.answered", "{{caller}} asked {{callee}}", vars),
		pending: t(
			"exchange.pending",
			"{{caller}} asked {{callee}} · waiting for the answer",
			vars,
		),
		waiting: t(
			"exchange.waiting",
			"{{caller}} wants to ask {{callee}} · needs your permission",
			vars,
		),
		denied: t(
			"exchange.denied",
			"{{caller}} was not allowed to ask {{callee}}",
			vars,
		),
		error: t("exchange.error", "{{caller}} could not ask {{callee}}", vars),
	}[exchange.state];
	const tone = TONES[exchange.state];
	const hasBody = Boolean(
		exchange.question ||
			exchange.answer ||
			exchange.note ||
			(exchange.conversationId && onOpen),
	);
	return (
		<RequestCard
			aria-label={headline}
			data-state={exchange.state}
			tone={tone}
			live={exchange.state === "pending" || exchange.state === "waiting"}
			className="my-2 text-sm"
		>
			<RequestCardHeader
				dense
				mark={<RequestMark icon={Icon} tone={tone} size="sm" />}
				title={headline}
				className={hasBody ? undefined : "pb-3"}
			/>
			{hasBody && (
				<RequestCardBody dense>
					{exchange.question && (
						<p className="line-clamp-3 rounded-lg bg-muted/50 px-3 py-2 text-[13px] leading-relaxed text-muted-foreground">
							{exchange.question}
						</p>
					)}
					{exchange.answer && (
						<div className="space-y-1 border-l-2 border-primary/40 pl-3">
							<span className="block text-xs font-medium text-muted-foreground">
								{t("exchange.answeredBy", "{{callee}} answered", vars)}
							</span>
							<p className="whitespace-pre-wrap leading-relaxed">
								{exchange.answer}
							</p>
						</div>
					)}
					{exchange.note && (
						<p className="text-xs leading-relaxed text-muted-foreground">
							{exchange.note}
						</p>
					)}
					{exchange.conversationId && onOpen && (
						<ExchangeLink
							conversationId={exchange.conversationId}
							onOpen={onOpen}
						/>
					)}
				</RequestCardBody>
			)}
		</RequestCard>
	);
}

/** The exchange conversations listed under a bot; the owner reads them, nothing is sent from here. */
export function BotExchangeList({
	entries,
	nameOf,
	onOpen,
}: Readonly<{
	entries: ReadonlyArray<BotExchangeEntry>;
	nameOf: (botId: string) => string;
	onOpen: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	if (entries.length === 0) {
		return (
			<p className="text-sm text-muted-foreground">
				{t("exchange.none", "This bot has not talked to your other bots yet.")}
			</p>
		);
	}
	return (
		<ul className="flex flex-col gap-1">
			{entries.map((entry) => (
				<li key={entry.id}>
					<button
						type="button"
						onClick={() => onOpen(entry.id)}
						className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-sm hover:bg-muted"
					>
						<ArrowRightLeftIcon className="size-4 shrink-0" aria-hidden />
						<span className="min-w-0 flex-1 truncate">
							{t("exchange.with", "With {{name}}", {
								name: nameOf(entry.otherBotId),
							})}
						</span>
						<span className="text-xs text-muted-foreground">
							{t("exchange.readOnly", "Read only")}
						</span>
					</button>
				</li>
			))}
		</ul>
	);
}
