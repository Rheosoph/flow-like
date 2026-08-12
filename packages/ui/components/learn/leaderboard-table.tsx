"use client";
import { motion } from "framer-motion";
import { Crown, Medal, Sparkles, Trophy } from "lucide-react";
import { useMemo } from "react";
import type { LeaderboardEntry } from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { EmptyState } from "../ui/empty-state";

interface LeaderboardTableProps {
	readonly entries: ReadonlyArray<LeaderboardEntry>;
	readonly currentUserId?: string;
}

function initials(name: string): string {
	return (
		name
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((part) => part[0]?.toUpperCase())
			.join("") || "U"
	);
}

function LeaderboardAvatar({
	entry,
	className,
}: {
	readonly entry: LeaderboardEntry;
	readonly className?: string;
}) {
	return (
		<Avatar className={cn("border border-border/50 bg-muted", className)}>
			{entry.avatar_url ? (
				<AvatarImage src={entry.avatar_url} className="object-cover" />
			) : null}
			<AvatarFallback className="text-xs font-semibold">
				{initials(entry.display_name)}
			</AvatarFallback>
		</Avatar>
	);
}

export function LeaderboardTable({
	entries,
	currentUserId,
}: LeaderboardTableProps) {
	const top = useMemo(() => entries.slice(0, 3), [entries]);
	const rest = useMemo(() => entries.slice(3), [entries]);
	const myEntry = useMemo(
		() => entries.find((e) => e.user_id === currentUserId) ?? null,
		[entries, currentUserId],
	);
	const myRank = useMemo(() => {
		if (!myEntry) return null;
		return entries.findIndex((e) => e.user_id === myEntry.user_id) + 1;
	}, [entries, myEntry]);

	if (entries.length === 0) {
		return (
			<div className="flex justify-center py-6">
				<EmptyState
					title="The leaderboard is empty"
					description={
						"Opt in above to be the first.\nPoints come from completing\nchallenges and lessons."
					}
					icons={[Trophy, Medal, Sparkles]}
				/>
			</div>
		);
	}

	return (
		<section className="space-y-6">
			{/* podium */}
			<div className="grid grid-cols-3 gap-3 md:gap-4 items-end">
				{[1, 0, 2].map((podiumIndex, displayIndex) => {
					const entry = top[podiumIndex];
					if (!entry) return <div key={displayIndex} />;
					const place = podiumIndex + 1;
					const isMe = currentUserId === entry.user_id;
					const heights = ["h-32 md:h-40", "h-44 md:h-56", "h-28 md:h-36"];
					const tones: ReadonlyArray<{
						readonly bg: string;
						readonly ring: string;
						readonly icon: typeof Trophy;
						readonly iconColor: string;
					}> = [
						{
							bg: "from-yellow-400/30 via-amber-500/20 to-orange-500/10",
							ring: "ring-yellow-500/40",
							icon: Crown,
							iconColor: "text-yellow-500",
						},
						{
							bg: "from-slate-300/20 via-zinc-400/15 to-slate-500/10",
							ring: "ring-slate-400/40",
							icon: Medal,
							iconColor: "text-slate-300",
						},
						{
							bg: "from-amber-700/20 via-orange-700/15 to-red-700/10",
							ring: "ring-amber-700/40",
							icon: Medal,
							iconColor: "text-amber-700 dark:text-amber-600",
						},
					];
					const tone = tones[place - 1];
					const Icon = tone.icon;
					return (
						<motion.div
							key={entry.user_id}
							initial={{ opacity: 0, y: 12 }}
							animate={{ opacity: 1, y: 0 }}
							transition={{
								duration: 0.45,
								delay: displayIndex * 0.08,
								ease: "easeOut",
							}}
							className={cn(
								"relative overflow-hidden rounded-2xl border bg-linear-to-br backdrop-blur-md p-4 md:p-5 flex flex-col justify-end",
								tone.bg,
								tone.ring,
								"ring-1 border-border/40",
								heights[place - 1],
								isMe && "shadow-[0_0_0_2px_var(--color-primary)]",
							)}
						>
							<div className="absolute top-3 right-3">
								<Icon className={cn("size-5", tone.iconColor)} />
							</div>
							<div className="absolute top-3 left-3 text-xs font-mono text-foreground/60">
								#{place}
							</div>
							<div className="space-y-0.5">
								<LeaderboardAvatar entry={entry} className="mb-2 size-9" />
								<div className="text-sm font-semibold truncate">
									{entry.display_name}
								</div>
								<div className="text-2xl font-bold tabular-nums">
									{entry.total_points.toLocaleString()}
								</div>
								<div className="text-[11px] uppercase tracking-wide text-muted-foreground">
									points
								</div>
							</div>
						</motion.div>
					);
				})}
			</div>

			{/* my rank pin */}
			{myEntry && myRank && myRank > 3 && (
				<MyRankRow entry={myEntry} rank={myRank} />
			)}

			{/* rest */}
			{rest.length > 0 && (
				<ol className="rounded-2xl border border-border/50 bg-card/60 backdrop-blur-sm divide-y divide-border/40 overflow-hidden">
					{rest.map((entry, i) => {
						const place = i + 4;
						const isMe = currentUserId === entry.user_id;
						return (
							<motion.li
								key={entry.user_id}
								initial={{ opacity: 0 }}
								animate={{ opacity: 1 }}
								transition={{ delay: i * 0.03 }}
								className={cn(
									"flex items-center gap-4 px-4 py-3 transition-colors hover:bg-muted/40",
									isMe && "bg-primary/5",
								)}
							>
								<span className="w-10 text-sm font-mono text-muted-foreground tabular-nums">
									#{place}
								</span>
								<LeaderboardAvatar entry={entry} className="size-8" />
								<span className="flex-1 font-medium truncate">
									{entry.display_name}
								</span>
								{isMe && (
									<span className="text-[10px] font-medium uppercase tracking-wide text-primary">
										You
									</span>
								)}
								<span className="font-mono text-sm tabular-nums text-foreground/80">
									{entry.total_points.toLocaleString()}
								</span>
							</motion.li>
						);
					})}
				</ol>
			)}
		</section>
	);
}

function MyRankRow({
	entry,
	rank,
}: {
	readonly entry: LeaderboardEntry;
	readonly rank: number;
}) {
	return (
		<div className="rounded-2xl border border-primary/30 bg-primary/5 backdrop-blur-sm px-4 py-3 flex items-center gap-4">
			<div className="size-9 rounded-xl bg-primary/15 grid place-items-center">
				<LeaderboardAvatar entry={entry} className="size-9" />
			</div>
			<div className="flex-1 min-w-0">
				<div className="text-xs uppercase tracking-wide text-muted-foreground">
					You're at
				</div>
				<div className="font-medium truncate">
					#{rank} · {entry.display_name}
				</div>
			</div>
			<div className="font-mono tabular-nums text-foreground">
				{entry.total_points.toLocaleString()} pts
			</div>
		</div>
	);
}
