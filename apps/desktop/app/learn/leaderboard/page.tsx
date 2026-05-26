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
import type { LeaderboardOptIn } from "@flow-like/flow-like-ui/lib/learn/types";
import { motion } from "framer-motion";
import { Eye, EyeOff, Pencil, Trophy } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
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
	const sub = auth.user?.profile.sub;

	const meKey = useMemo(
		() => ["learn", "leaderboard", "me", profileId, sub] as const,
		[profileId, sub],
	);

	const leaderboardQuery = useQuery({
		queryKey: ["learn", "leaderboard", profileId],
		enabled: Boolean(profile),
		queryFn: () => learnApi.getLeaderboard(profile!, auth, { limit: 50 }),
	});

	const myOptInQuery = useQuery({
		queryKey: meKey,
		enabled: Boolean(profile && sub),
		queryFn: () => learnApi.getMyOptIn(profile!, auth),
	});

	const fallbackName =
		(typeof auth.user?.profile?.preferred_username === "string"
			? auth.user.profile.preferred_username
			: typeof auth.user?.profile?.name === "string"
				? auth.user.profile.name
				: null) ?? "Anonymous";

	const me = myOptInQuery.data;
	const optedIn = Boolean(me?.is_opted_in);
	const displayName = me?.display_name?.trim() || fallbackName;

	const updateMutation = useMutation({
		mutationFn: (next: { display_name: string; is_opted_in: boolean }) =>
			learnApi.updateMyOptIn(profile!, auth, next),
		onMutate: async (next) => {
			await queryClient.cancelQueries({ queryKey: meKey });
			const previous = queryClient.getQueryData<LeaderboardOptIn | null>(meKey);
			queryClient.setQueryData<LeaderboardOptIn | null>(meKey, (current) => ({
				...(current ?? {
					user_id: sub ?? "",
					total_points: 0,
				}),
				display_name: next.display_name,
				is_opted_in: next.is_opted_in,
			}));
			return { previous };
		},
		onError: (err, _next, ctx) => {
			console.error("opt-in update failed", err);
			if (ctx) {
				queryClient.setQueryData(meKey, ctx.previous);
			}
			toast.error("Could not save preferences. Try again.");
		},
		onSuccess: (saved) => {
			queryClient.setQueryData<LeaderboardOptIn | null>(meKey, saved);
			toast.success("Preferences saved");
		},
		onSettled: () => {
			queryClient.invalidateQueries({ queryKey: ["learn", "leaderboard"] });
		},
	});

	const toggleOptIn = (next: boolean) => {
		if (!profile) return;
		updateMutation.mutate({
			display_name: displayName,
			is_opted_in: next,
		});
	};

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-3xl p-6 md:p-10 space-y-6">
				{/* Compact header */}
				<motion.section
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.4, ease: "easeOut" }}
					className="flex items-end gap-4"
				>
					<div className="flex-1">
						<h1 className="text-2xl md:text-3xl font-semibold tracking-tight inline-flex items-center gap-2">
							<Trophy className="size-6 text-yellow-500" />
							Leaderboard
						</h1>
						<p className="text-sm text-muted-foreground mt-0.5">
							Earn points by solving challenges and completing lessons.
						</p>
					</div>
				</motion.section>

				{/* Compact opt-in row */}
				<motion.section
					initial={{ opacity: 0, y: 8 }}
					animate={{ opacity: 1, y: 0 }}
					transition={{ duration: 0.4, delay: 0.05, ease: "easeOut" }}
					className="rounded-xl border border-border/50 bg-card/60 backdrop-blur-sm px-4 py-3 flex items-center gap-3"
				>
					<div className="size-8 rounded-lg bg-muted/60 grid place-items-center shrink-0">
						{optedIn ? (
							<Eye className="size-4 text-primary" />
						) : (
							<EyeOff className="size-4 text-muted-foreground" />
						)}
					</div>
					<div className="flex-1 min-w-0">
						<div className="text-sm font-medium truncate">
							{optedIn ? "Visible as " : "Hidden — would appear as "}
							<span className="text-foreground">{displayName}</span>
						</div>
						<div className="text-xs text-muted-foreground">
							{optedIn
								? "You're on the public leaderboard."
								: "Flip the switch to compete publicly."}
						</div>
					</div>
					<EditNameButton
						currentName={displayName}
						optedIn={optedIn}
						pending={updateMutation.isPending}
						onSave={(name) =>
							updateMutation.mutate({
								display_name: name,
								is_opted_in: optedIn,
							})
						}
					/>
					<Switch
						checked={optedIn}
						onCheckedChange={toggleOptIn}
						disabled={updateMutation.isPending || !profile}
					/>
				</motion.section>

				{/* Leaderboard front and center */}
				<LeaderboardTable
					entries={leaderboardQuery.data ?? []}
					currentUserId={sub}
				/>
			</div>
		</div>
	);
}

function EditNameButton({
	currentName,
	optedIn,
	pending,
	onSave,
}: {
	readonly currentName: string;
	readonly optedIn: boolean;
	readonly pending: boolean;
	readonly onSave: (name: string) => void;
}) {
	const [open, setOpen] = useState(false);
	const [draft, setDraft] = useState(currentName);
	useEffect(() => setDraft(currentName), [currentName, open]);

	if (!open) {
		return (
			<Button
				variant="ghost"
				size="sm"
				className="h-8 px-2 text-muted-foreground hover:text-foreground"
				onClick={() => setOpen(true)}
				title="Change display name"
			>
				<Pencil className="size-3.5" />
			</Button>
		);
	}

	return (
		<div className="flex items-center gap-2">
			<Label htmlFor="display-name" className="sr-only">
				Display name
			</Label>
			<Input
				id="display-name"
				value={draft}
				onChange={(e) => setDraft(e.target.value)}
				maxLength={40}
				placeholder={optedIn ? "Public name" : "Display name"}
				className="h-8 w-44 rounded-lg text-sm"
				autoFocus
			/>
			<Button
				size="sm"
				className="h-8 rounded-lg"
				disabled={pending || draft.trim().length === 0}
				onClick={() => {
					onSave(draft.trim() || currentName);
					setOpen(false);
				}}
			>
				Save
			</Button>
			<Button
				variant="ghost"
				size="sm"
				className="h-8 rounded-lg"
				onClick={() => setOpen(false)}
			>
				Cancel
			</Button>
		</div>
	);
}
