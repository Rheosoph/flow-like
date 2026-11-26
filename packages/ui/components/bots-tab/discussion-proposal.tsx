"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, MessagesSquareIcon } from "lucide-react";
import { useState } from "react";
import {
	RequestCard,
	RequestCardBody,
	RequestCardFooter,
	RequestCardHeader,
	RequestDetails,
	RequestMark,
} from "../interfaces/chat-default/request-card";
import { formatCost } from "../interfaces/chat-default/usage-breakdown";
import { Button } from "../ui/button";
import type { DiscussionEstimate } from "./discussion-proposal-model";

/**
 * A bot's request for a longer discussion, under its message: the rounds, why, what they may
 * cost, and the button that allows them. Once someone allowed it, it says who.
 */
export function DiscussionProposalCard({
	botName,
	rounds,
	reason,
	estimate,
	allowedBy,
	limited,
	onAllow,
}: Readonly<{
	botName: string;
	rounds: number;
	reason: string;
	estimate: DiscussionEstimate;
	/** Who allowed it and for how many rounds, once someone did; `name` is `null` for the reader. */
	allowedBy?: { name: string | null; rounds: number };
	/** Some bot here isn't the reader's: their turns count toward the reader's daily limit. */
	limited: boolean;
	/** Absent when the reader can't write here. */
	onAllow?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [sent, setSent] = useState(false);
	const settled = Boolean(allowedBy);
	const tone = settled ? "muted" : "primary";
	const cost =
		estimate.total === null
			? t(
					"discussion.costUnknown",
					"No reply here has reported its model cost yet.",
				)
			: t(
					"discussion.cost",
					"About {{cost}} in model costs, at the price of the replies so far.",
					{ cost: formatCost(estimate.total) },
				);
	return (
		<RequestCard
			tone={tone}
			live={!settled}
			className="mt-2 max-w-xl"
			data-slot="discussion-proposal"
		>
			<RequestCardHeader
				dense
				tone={tone}
				live={!settled}
				mark={
					<RequestMark
						icon={settled ? CheckIcon : MessagesSquareIcon}
						tone={tone}
						size="sm"
					/>
				}
				eyebrow={t(
					"discussion.eyebrow",
					"{{bot}} suggests a longer discussion",
					{
						bot: botName,
					},
				)}
				title={t(
					"discussion.title",
					"Up to {{rounds}} rounds, {{messages}} messages",
					{ rounds, messages: rounds * 2 },
				)}
			/>
			<RequestCardBody dense>
				{reason && <RequestDetails text={reason} />}
				<p className="text-xs leading-relaxed text-muted-foreground">
					{cost}{" "}
					{t("discussion.payer", "Each bot's owner pays for their own bot.")}
					{limited &&
						` ${t(
							"discussion.limit",
							"Turns of bots you don't own count toward your 20 a day with each.",
						)}`}
				</p>
			</RequestCardBody>
			<RequestCardFooter
				start={
					allowedBy ? (
						<span className="text-xs text-muted-foreground">
							{allowedBy.name === null
								? t(
										"discussion.allowedByYou",
										"You allowed {{rounds}} rounds",
										{
											rounds: allowedBy.rounds,
										},
									)
								: t(
										"discussion.allowed",
										"{{name}} allowed {{rounds}} rounds",
										{
											name: allowedBy.name,
											rounds: allowedBy.rounds,
										},
									)}
						</span>
					) : (
						<span className="text-xs text-muted-foreground">
							{t(
								"discussion.waiting",
								"The bots wait until someone allows it.",
							)}
						</span>
					)
				}
			>
				{!settled && onAllow && (
					<Button
						size="sm"
						disabled={sent}
						onClick={() => {
							setSent(true);
							onAllow();
						}}
					>
						{t("discussion.allow", "Allow {{rounds}} rounds", { rounds })}
					</Button>
				)}
			</RequestCardFooter>
		</RequestCard>
	);
}
