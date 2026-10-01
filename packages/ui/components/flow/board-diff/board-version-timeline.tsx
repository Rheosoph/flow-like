"use client";

import { useTranslation } from "@flow-like/locales";
import { EyeIcon, PencilLineIcon } from "lucide-react";
import { memo } from "react";
import { useUserIdentity } from "../../../hooks/use-user-lookup";
import type {
	BoardVersion,
	IBoardVersionInfo,
} from "../../../lib/schema/flow/board-version";
import { cn } from "../../../lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { UserAvatar } from "../../ui/user-identity";

export type CompareRef =
	| { kind: "draft" }
	| { kind: "last-visit" }
	| { kind: "version"; version: BoardVersion };

export const compareRefKey = (ref: CompareRef) =>
	ref.kind === "version" ? ref.version.join(".") : ref.kind;

export type VersionBump = "Major" | "Minor" | "Patch";

export interface TimelineStop {
	key: string;
	ref: CompareRef;
	/** Unix ms. */
	at?: number;
	author?: string | null;
	bump?: VersionBump;
}

function bumpBetween(
	previous: BoardVersion | undefined,
	current: BoardVersion,
): VersionBump {
	if (!previous || previous[0] !== current[0]) return "Major";
	if (previous[1] !== current[1]) return "Minor";
	return "Patch";
}

/** Published versions oldest first, the last visit where it falls in time, the draft last. */
export function buildTimeline(
	versions: IBoardVersionInfo[],
	lastSeenAt: number | undefined,
	draftAt: number | undefined,
): TimelineStop[] {
	const ordered = [...versions].sort(
		(a, b) =>
			a.version[0] - b.version[0] ||
			a.version[1] - b.version[1] ||
			a.version[2] - b.version[2],
	);
	const stops: TimelineStop[] = ordered.map((info, index) => ({
		key: info.version.join("."),
		ref: { kind: "version", version: info.version },
		at: info.published_at ?? undefined,
		author: info.published_by,
		bump: bumpBetween(ordered[index - 1]?.version, info.version),
	}));
	if (lastSeenAt !== undefined) {
		const visit: TimelineStop = {
			key: "last-visit",
			ref: { kind: "last-visit" },
			at: lastSeenAt,
		};
		const after = stops.findIndex(
			(stop) => stop.at !== undefined && stop.at > lastSeenAt,
		);
		stops.splice(after === -1 ? stops.length : after, 0, visit);
	}
	stops.push({ key: "draft", ref: { kind: "draft" }, at: draftAt });
	return stops;
}

export function useStopLabel() {
	const { t } = useTranslation("flow");
	return (stop: TimelineStop) =>
		stop.ref.kind === "draft"
			? t("boardDiffDraft", "Draft")
			: stop.ref.kind === "last-visit"
				? t("boardDiffLastVisit", "Your last visit")
				: `v${stop.ref.version.join(".")}`;
}

function formatShortDate(at: number) {
	const date = new Date(at);
	const today = new Date();
	const sameDay = date.toDateString() === today.toDateString();
	return sameDay
		? date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
		: date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

const StopAuthor = memo(function StopAuthor({
	userId,
}: Readonly<{ userId: string }>) {
	const identity = useUserIdentity(userId);
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span>
					<UserAvatar
						avatarUrl={identity.avatarUrl}
						initials={identity.initials}
						label={identity.label}
						size="xs"
						className="size-4 text-[7px]"
					/>
				</span>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="text-xs">
				{identity.label}
			</TooltipContent>
		</Tooltip>
	);
});

export const BoardVersionTimeline = memo(function BoardVersionTimeline({
	stops,
	baseKey,
	headKey,
	onPick,
}: Readonly<{
	stops: TimelineStop[];
	baseKey: string;
	headKey: string;
	onPick: (stop: TimelineStop) => void;
}>) {
	const { t } = useTranslation("flow");
	const label = useStopLabel();
	const baseIndex = stops.findIndex((s) => s.key === baseKey);
	const headIndex = stops.findIndex((s) => s.key === headKey);
	const lo = Math.min(baseIndex, headIndex);
	const hi = Math.max(baseIndex, headIndex);

	return (
		<div className="overflow-x-auto rounded-lg border bg-card [scrollbar-width:thin]">
			<ol className="grid min-w-max auto-cols-[minmax(7.5rem,1fr)] grid-flow-col px-1 pb-2 pt-1">
				{stops.map((stop, index) => {
					const role =
						stop.key === baseKey
							? "base"
							: stop.key === headKey
								? "head"
								: undefined;
					const inRange = index >= lo && index <= hi;
					return (
						<li key={stop.key} className="relative">
							<span
								aria-hidden
								className={cn(
									"absolute left-0 right-1/2 top-[41px] h-0.5 bg-border",
									index === 0 && "hidden",
									index > lo && index <= hi && "h-[3px] bg-primary",
								)}
							/>
							<span
								aria-hidden
								className={cn(
									"absolute left-1/2 right-0 top-[41px] h-0.5 bg-border",
									index === stops.length - 1 && "hidden",
									index >= lo && index < hi && "h-[3px] bg-primary",
								)}
							/>
							<button
								type="button"
								aria-pressed={Boolean(role)}
								onClick={() => onPick(stop)}
								className={cn(
									"relative grid w-full justify-items-center gap-0.5 rounded-md px-2 pb-1 pt-6 text-center outline-none hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring",
									!inRange && "opacity-70",
								)}
							>
								{role && (
									<span
										className={cn(
											"absolute left-1/2 top-0.5 -translate-x-1/2 rounded-full px-1.5 py-0.5 text-[9.5px] font-semibold uppercase leading-none tracking-wider",
											role === "base"
												? "bg-card text-primary ring-1 ring-primary"
												: "bg-primary text-primary-foreground",
										)}
									>
										{role === "base"
											? t("boardDiffBase", "Base")
											: t("boardDiffCompare", "Compare")}
									</span>
								)}
								<span className="relative z-10 grid size-8 place-items-center">
									{stop.ref.kind === "last-visit" ? (
										<span
											className={cn(
												"grid size-5 place-items-center rounded-full border bg-card text-muted-foreground",
												role && "border-primary text-primary",
											)}
										>
											<EyeIcon className="size-3" />
										</span>
									) : stop.ref.kind === "draft" ? (
										<span
											className={cn(
												"grid size-5 place-items-center rounded-full border border-dashed border-primary bg-card text-primary",
												role &&
													"border-solid bg-primary text-primary-foreground",
											)}
										>
											<PencilLineIcon className="size-3" />
										</span>
									) : (
										<span
											className={cn(
												"block rounded-full border-2 border-muted-foreground bg-card",
												stop.bump === "Major" &&
													"size-[18px] border-foreground bg-foreground",
												stop.bump === "Minor" && "size-3.5 bg-muted-foreground",
												stop.bump === "Patch" && "size-2.5",
												role && "border-primary bg-primary",
												role === "base" && "bg-card",
											)}
										/>
									)}
								</span>
								<span
									className={cn(
										"text-xs font-semibold",
										stop.ref.kind === "version" && "font-mono",
									)}
								>
									{label(stop)}
								</span>
								<span className="flex items-center gap-1 text-[11px] text-muted-foreground">
									{stop.author && <StopAuthor userId={stop.author} />}
									{stop.at !== undefined
										? formatShortDate(stop.at)
										: stop.ref.kind === "draft"
											? t("boardDiffUnpublished", "unpublished")
											: ""}
								</span>
							</button>
						</li>
					);
				})}
			</ol>
		</div>
	);
});
