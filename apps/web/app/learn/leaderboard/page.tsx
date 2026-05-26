"use client";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Button,
	Input,
	Label,
	LeaderboardTable,
	Switch,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { motion } from "framer-motion";
import { Eye, EyeOff, Trophy } from "lucide-react";
import { useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { learnApi } from "../../../lib/learn-api";

export default function LeaderboardPage() {
	const auth = useAuth();
	const queryClient = useQueryClient();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";

	const leaderboardQuery = useQuery({
		queryKey: ["learn", "leaderboard", profileId],
		enabled: Boolean(profile),
		queryFn: () => learnApi.getLeaderboard(profile!, auth, { limit: 50 }),
	});

	const myOptInQuery = useQuery({
		queryKey: ["learn", "leaderboard", "me", profileId, auth.user?.profile.sub],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.getMyOptIn(profile!, auth),
	});

	const [displayName, setDisplayName] = useState("");
	const [optedIn, setOptedIn] = useState(false);

	useEffect(() => {
		if (myOptInQuery.data) {
			setDisplayName(myOptInQuery.data.display_name ?? "");
			setOptedIn(Boolean(myOptInQuery.data.is_opted_in));
		} else if (auth.user?.profile?.preferred_username) {
			setDisplayName(String(auth.user.profile.preferred_username));
		}
	}, [myOptInQuery.data, auth.user]);

	const updateMutation = useMutation({
		mutationFn: () =>
			learnApi.updateMyOptIn(profile!, auth, {
				display_name: displayName.trim() || "Anonymous",
				is_opted_in: optedIn,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["learn", "leaderboard"] });
		},
	});

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-3xl p-6 md:p-10 space-y-8">
				{/* Hero */}
				<motion.section
					initial={{ opacity: 0, y: 12 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.45, ease: "easeOut" }}
					className="text-center space-y-3 pt-4"
				>
					<div className="inline-flex items-center justify-center size-14 rounded-2xl bg-linear-to-br from-yellow-400/20 to-amber-600/10 ring-1 ring-yellow-500/30">
						<Trophy className="size-7 text-yellow-500" />
					</div>
					<h1 className="text-3xl md:text-4xl font-semibold tracking-tight">
						Leaderboard
					</h1>
					<p className="text-muted-foreground">
						Earn points by solving challenges and completing lessons.
					</p>
				</motion.section>

				{/* Opt-in */}
				<motion.section
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.45, delay: 0.05, ease: "easeOut" }}
					className="rounded-2xl border border-border/50 bg-card/70 backdrop-blur-sm p-5 md:p-6 space-y-4"
				>
					<div className="flex items-start gap-3">
						<div className="size-10 rounded-xl bg-primary/10 grid place-items-center shrink-0">
							{optedIn ? (
								<Eye className="size-5 text-primary" />
							) : (
								<EyeOff className="size-5 text-muted-foreground" />
							)}
						</div>
						<div className="flex-1">
							<h2 className="font-semibold">Show me on the leaderboard</h2>
							<p className="text-sm text-muted-foreground">
								Off by default — flip the switch to start competing.
							</p>
						</div>
						<Switch
							checked={optedIn}
							onCheckedChange={setOptedIn}
							id="leaderboard-opt-in"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="display-name" className="text-xs uppercase tracking-wide text-muted-foreground">
							Display name
						</Label>
						<Input
							id="display-name"
							value={displayName}
							onChange={(e) => setDisplayName(e.target.value)}
							maxLength={40}
							placeholder="How you appear to others"
							className="rounded-xl"
						/>
					</div>
					<Button
						onClick={() => updateMutation.mutate()}
						disabled={updateMutation.isPending || !profile}
						className="rounded-xl"
					>
						Save preferences
					</Button>
				</motion.section>

				{/* Leaderboard */}
				<LeaderboardTable
					entries={leaderboardQuery.data ?? []}
					currentUserId={auth.user?.profile.sub}
				/>
			</div>
		</div>
	);
}
