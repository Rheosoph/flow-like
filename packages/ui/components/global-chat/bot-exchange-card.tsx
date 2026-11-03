"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRightLeftIcon,
	ClockIcon,
	ExternalLinkIcon,
	ShieldQuestionIcon,
	XCircleIcon,
} from "lucide-react";
import type { BotExchangeEntry, BotExchangeView } from "./bot-exchange";

const ICONS = {
	answered: ArrowRightLeftIcon,
	pending: ClockIcon,
	waiting: ShieldQuestionIcon,
	denied: XCircleIcon,
	error: XCircleIcon,
} as const;

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
			className="inline-flex items-center gap-1 text-xs text-primary underline-offset-2 hover:underline"
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
	return (
		<section
			aria-label={headline}
			data-state={exchange.state}
			className="my-2 flex flex-col gap-2 rounded-xl border bg-card p-3 text-sm"
		>
			<header className="flex items-center gap-2 font-medium">
				<Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
				<span className="min-w-0 truncate">{headline}</span>
			</header>
			{exchange.question && (
				<p className="line-clamp-3 text-muted-foreground">
					{exchange.question}
				</p>
			)}
			{exchange.answer && (
				<div className="flex flex-col gap-1 border-l-2 pl-3">
					<span className="text-xs text-muted-foreground">
						{t("exchange.answeredBy", "{{callee}} answered", vars)}
					</span>
					<p className="whitespace-pre-wrap">{exchange.answer}</p>
				</div>
			)}
			{exchange.note && (
				<p className="text-xs text-muted-foreground">{exchange.note}</p>
			)}
			{exchange.conversationId && onOpen && (
				<ExchangeLink
					conversationId={exchange.conversationId}
					onOpen={onOpen}
				/>
			)}
		</section>
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
