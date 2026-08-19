"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	ClockIcon,
	UserCheckIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useCallback } from "react";
import { toast } from "sonner";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	EmptyState,
	type IJoinRequest,
	Skeleton,
	useBackend,
	useInfiniteInvoke,
	useInvoke,
} from "../../../";
import {
	userAvatarUrl,
	userDisplayName,
	userHandle,
	userInitials,
} from "../../../lib/user-display";
import {
	SectionHeading,
	StatusChip,
	TEAM_ROW_HANDLE,
	TEAM_ROW_META,
	TEAM_ROW_TITLE,
	TeamRowActions,
	TeamRowNote,
	TeamSection,
	teamRowClass,
} from "./team-shared";

export function TeamJoinManagement({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const {
		data: requestsPages,
		isLoading,
		fetchNextPage,
		refetch,
		hasNextPage,
	} = useInfiniteInvoke(backend.teamState.getJoinRequests, backend.teamState, [
		appId,
	]);

	const requests = requestsPages?.pages.flat() ?? [];

	return (
		<TeamSection>
			<SectionHeading
				icon={ClockIcon}
				title={t("joinRequests", "Join requests")}
				count={requests.length}
				countTone={requests.length > 0 ? "attention" : "neutral"}
				description={t(
					"peopleWhoAskedToJoinApprovingAddsThemWithTheDefaultRole",
					"People who asked to join. Approving adds them with the default role.",
				)}
			/>

			{requests.length === 0 ? (
				<EmptyState
					className="max-w-full"
					title={t("noPendingRequests", "No pending requests")}
					description={t(
						"allJoinRequestsHaveBeenProcessed",
						"All join requests have been processed",
					)}
					icons={[UsersIcon, ClockIcon, UserCheckIcon]}
				/>
			) : (
				<div className="flex flex-col gap-2">
					{requests.map((request) => (
						<RequestRow
							key={request.id}
							request={request}
							appId={appId}
							refresh={async () => {
								await refetch();
							}}
						/>
					))}
					{hasNextPage && (
						<Button
							variant="outline"
							className="w-full"
							onClick={() => fetchNextPage()}
							disabled={isLoading}
						>
							{isLoading
								? "Loading..."
								: t("loadMoreRequests", "Load More Requests")}
						</Button>
					)}
				</div>
			)}
		</TeamSection>
	);
}

function RequestRow({
	appId,
	request,
	refresh,
}: Readonly<{ appId: string; request: IJoinRequest; refresh: () => void }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const user = useInvoke(backend.userState.lookupUser, backend.userState, [
		request.user_id,
	]);
	const userData = user.data;

	const acceptRequest = useCallback(async () => {
		try {
			await backend.teamState.acceptJoinRequest(appId, request.id);
			refresh();
		} catch (error) {
			console.error("Failed to accept request:", error);
			toast.error("Failed to accept request try again later");
		}
	}, [backend, appId, request.id, refresh]);

	const declineRequest = useCallback(async () => {
		try {
			await backend.teamState.rejectJoinRequest(appId, request.id);
			refresh();
		} catch (error) {
			console.error("Failed to decline request:", error);
			toast.error("Failed to decline request try again later");
		}
	}, [backend, appId, request.id, refresh]);

	if (!userData) {
		return (
			<div className={teamRowClass({ attention: true, align: "start" })}>
				<Skeleton className="size-9 shrink-0 rounded-full" />
				<div className="min-w-0 flex-1 space-y-2">
					<Skeleton className="h-4 w-45" />
					<Skeleton className="h-3 w-30" />
				</div>
				<TeamRowActions always>
					<Skeleton className="h-8 w-24" />
					<Skeleton className="h-8 w-24" />
				</TeamRowActions>
			</div>
		);
	}

	const evaluatedName = userDisplayName(userData, "Unknown User");
	const contact =
		userData.email ??
		userHandle(userData) ??
		t("noContactDetails", "No contact details");

	return (
		<div className={teamRowClass({ attention: true, align: "start" })}>
			<Avatar className="size-9 shrink-0">
				<AvatarImage src={userAvatarUrl(userData)} alt={evaluatedName} />
				<AvatarFallback className="text-[11px] font-semibold text-foreground">
					{userInitials(userData)}
				</AvatarFallback>
			</Avatar>

			<div className="min-w-0 flex-1">
				<div className={TEAM_ROW_TITLE}>
					<span className="truncate">{evaluatedName}</span>
					<span className={`${TEAM_ROW_HANDLE} truncate`}>{contact}</span>
					<StatusChip tone="attention" pip>
						{t("wantsToJoin", "Wants to join")}
					</StatusChip>
				</div>

				<div className={TEAM_ROW_META}>
					<span>
						{t("asked", "Asked")}{" "}
						{new Date(Date.parse(request.created_at)).toLocaleDateString(
							"en-US",
							{
								month: "short",
								day: "numeric",
								year: "numeric",
							},
						)}
					</span>
				</div>

				{request.comment && <TeamRowNote>{request.comment}</TeamRowNote>}
			</div>

			<TeamRowActions always>
				<Button size="sm" onClick={acceptRequest}>
					<CheckIcon className="size-3.5" />
					{t("approve", "Approve")}
				</Button>
				<Button size="sm" variant="outline" onClick={declineRequest}>
					<XIcon className="size-3.5" />
					{t("decline", "Decline")}
				</Button>
			</TeamRowActions>
		</div>
	);
}
