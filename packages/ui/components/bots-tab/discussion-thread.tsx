"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowLeftRightIcon,
	CheckIcon,
	HourglassIcon,
	ScaleIcon,
} from "lucide-react";
import { Fragment } from "react";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import {
	RequestCard,
	RequestCardBody,
	RequestCardHeader,
	RequestMark,
} from "../interfaces/chat-default/request-card";
import type {
	DiscussionPlace,
	RoomDiscussion,
} from "./discussion-thread-model";
import type { CrewDirectory } from "./use-crew-directory";

const MAX_FACES = 3;
/** How long an idle partner still shows as next: a bot that stays silent never posts. */
const TURN_GRACE_MS = 2 * 60_000;

function Faces({
	bots,
	size,
	joined = false,
}: Readonly<{ bots: readonly string[]; size: number; joined?: boolean }>) {
	return (
		<span
			aria-hidden
			className={cn("flex shrink-0 items-center", !joined && "-space-x-2")}
		>
			{bots.slice(0, MAX_FACES).map((id, at) => (
				<Fragment key={id}>
					{joined && at > 0 && (
						<ArrowLeftRightIcon className="mx-0.5 size-3 text-muted-foreground/70" />
					)}
					<span className="rounded-full ring-2 ring-background">
						<IdentityAvatar subject={id} size={size} />
					</span>
				</Fragment>
			))}
		</span>
	);
}

function pairOf(bots: readonly string[], directory: CrewDirectory) {
	return {
		first: directory(bots[0] ?? "").name,
		second: directory(bots[1] ?? "").name,
	};
}

/** Above a discussion's first message: who discusses, and for how long at most. */
export function DiscussionHeader({
	discussion,
	directory,
}: Readonly<{ discussion: RoomDiscussion; directory: CrewDirectory }>) {
	const { t } = useTranslation("bots");
	return (
		<div
			data-slot="discussion-header"
			className="flex items-center gap-2.5 text-xs text-muted-foreground"
		>
			<span className="h-px flex-1 bg-border" />
			<span
				className="inline-flex min-w-0 max-w-[85%] items-center gap-2 rounded-full border border-border/70 bg-muted/40 py-0.5 pr-3 pl-1"
				title={t(
					"discussion.thread.between",
					"Discussion between {{first}} and {{second}}",
					pairOf(discussion.bots, directory),
				)}
			>
				<Faces bots={discussion.bots} size={18} joined />
				<span className="min-w-0 truncate">
					<span className="font-medium text-foreground">
						{t("discussion.thread.label", "Discussion")}
					</span>
					{" · "}
					{t("discussion.thread.upTo", "up to {{rounds}} rounds", {
						rounds: discussion.rounds,
					})}
				</span>
			</span>
			<span className="h-px flex-1 bg-border" />
		</div>
	);
}

/**
 * Under a discussion message: its round, with a track of one segment per round, each split into
 * the two bots' messages, filled up to this one.
 */
export function DiscussionRoundTrack({
	place,
}: Readonly<{ place: DiscussionPlace }>) {
	const { t } = useTranslation("bots");
	const { discussion, step } = place;
	const segments = Array.from({ length: discussion.rounds }, (_, round) => [
		round * 2 + 1,
		round * 2 + 2,
	]);
	return (
		<div
			data-slot="discussion-round"
			className="mt-1.5 flex items-center gap-2"
		>
			<span
				aria-hidden
				className="flex max-w-40 shrink items-center gap-0.5"
				style={{ width: `${discussion.rounds * 1.125}rem` }}
			>
				{segments.map(([first, second]) => (
					<span
						key={first}
						className="flex h-1.5 min-w-1.5 flex-1 gap-px overflow-hidden rounded-full"
					>
						{[first, second].map(
							(at) =>
								at !== undefined &&
								at <= discussion.messageLimit && (
									<span
										key={at}
										className={cn(
											"flex-1 transition-colors duration-300",
											at < step.index && "bg-muted-foreground/45",
											at === step.index && "bg-primary",
											at > step.index && "bg-foreground/10",
										)}
									/>
								),
						)}
					</span>
				))}
			</span>
			<span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
				{t("discussion.thread.round", "Round {{round}} of {{rounds}}", {
					round: Math.min(step.round, discussion.rounds),
					rounds: discussion.rounds,
				})}
			</span>
		</div>
	);
}

function OutcomeCard({
	discussion,
	directory,
	round,
}: Readonly<{
	discussion: RoomDiscussion;
	directory: CrewDirectory;
	round: number;
}>) {
	const { t } = useTranslation("bots");
	const agreed = discussion.status === "agreed";
	const tone = agreed ? "good" : "muted";
	const params = { round, rounds: discussion.rounds };
	return (
		<RequestCard
			tone={tone}
			className="mt-2 max-w-xl"
			data-slot="discussion-outcome"
			data-status={discussion.status}
		>
			<RequestCardHeader
				dense
				tone={tone}
				mark={
					<RequestMark
						icon={agreed ? CheckIcon : ScaleIcon}
						tone={tone}
						size="sm"
					>
						<Faces bots={discussion.bots.slice(0, 2)} size={20} />
					</RequestMark>
				}
				eyebrow={t(
					"discussion.thread.pair",
					"{{first}} and {{second}}",
					pairOf(discussion.bots, directory),
				)}
				title={
					agreed
						? t(
								"discussion.thread.agreed",
								"Agreed after {{round}} of {{rounds}} rounds",
								params,
							)
						: t(
								"discussion.thread.disagreed",
								"No agreement after {{round}} of {{rounds}} rounds",
								params,
							)
				}
			/>
			{discussion.outcome?.summary && (
				<RequestCardBody dense>
					<p className="text-[13px] leading-relaxed text-pretty text-foreground/90">
						{discussion.outcome.summary}
					</p>
				</RequestCardBody>
			)}
		</RequestCard>
	);
}

function TurnLine({
	name,
	active,
}: Readonly<{ name: string; active: boolean }>) {
	const { t } = useTranslation("bots");
	return (
		<p
			data-slot="discussion-turn"
			aria-live="polite"
			className="mt-2 flex items-center gap-2 text-xs text-muted-foreground"
		>
			<span
				aria-hidden
				className={cn(
					"size-1.5 shrink-0 rounded-full",
					active
						? "bg-primary motion-safe:animate-pulse-soft"
						: "bg-muted-foreground/50",
				)}
			/>
			{active
				? t("discussion.thread.replying", "{{name}} is replying", { name })
				: t("discussion.thread.turn", "{{name}}'s turn", { name })}
		</p>
	);
}

/**
 * Under a discussion's last message: how it ended, or whose turn it is. A proposal for more
 * rounds shows its own card instead.
 */
export function DiscussionEnd({
	place,
	directory,
	active,
	now = Date.now(),
}: Readonly<{
	place: DiscussionPlace;
	directory: CrewDirectory;
	/** Bots working on a reply in this conversation right now. */
	active: ReadonlySet<string>;
	now?: number;
}>) {
	const { t } = useTranslation("bots");
	const { discussion, step } = place;
	const round = Math.min(step.round, discussion.rounds);
	switch (discussion.status) {
		case "agreed":
		case "disagreed":
			return (
				<OutcomeCard
					discussion={discussion}
					directory={directory}
					round={round}
				/>
			);
		case "rounds_used":
			return (
				<p
					data-slot="discussion-outcome"
					data-status={discussion.status}
					className="mt-2 flex items-center gap-2 text-xs text-muted-foreground"
				>
					<HourglassIcon aria-hidden className="size-3.5 shrink-0" />
					<span>
						<span className="font-medium text-foreground">
							{t("discussion.thread.roundsUsed", "All {{rounds}} rounds used", {
								rounds: discussion.rounds,
							})}
						</span>
						{" · "}
						{t("discussion.thread.goOn", "Mention a bot to let them go on.")}
					</span>
				</p>
			);
		case "open": {
			const next = discussion.next;
			if (!next) return null;
			const working = active.has(next);
			const idle =
				discussion.superseded || now - discussion.lastAt > TURN_GRACE_MS;
			if (idle && !working) return null;
			return <TurnLine name={directory(next).name} active={working} />;
		}
		default:
			return null;
	}
}
