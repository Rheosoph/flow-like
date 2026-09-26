"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	ClockIcon,
	UserCheckIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	EmptyState,
	type IJoinRequest,
	RolePermissions,
	Skeleton,
	useBackend,
	useInfiniteInvoke,
	useInvoke,
} from "../../../";
import { apiErrorMessage } from "../../../lib/api-error";
import { asArray } from "../../../lib/response-shape";
import {
	userAvatarUrl,
	userDisplayName,
	userHandle,
	userInitials,
} from "../../../lib/user-display";
import { usePayments } from "../../payments/use-payments";
import { SectionLockedPanel } from "../permission";
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
	useTeamAccess,
} from "./team-shared";

export function TeamJoinManagement({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const access = useTeamAccess(appId);
	const app = useInvoke(backend.appState.getApp, backend.appState, [appId]);
	const paid = (app.data?.price ?? 0) > 0;
	const {
		data: requestsPages,
		isLoading,
		fetchNextPage,
		refetch,
		hasNextPage,
	} = useInfiniteInvoke(
		backend.teamState.getJoinRequests,
		backend.teamState,
		[appId],
		50,
		access.canAdminister && !access.isLoading,
	);

	const requests = requestsPages?.pages.flatMap((page) => asArray(page)) ?? [];

	if (!access.canAdminister && !access.isLoading) {
		return (
			<SectionLockedPanel
				feature={t("joinRequests", "Join requests")}
				description={
					paid
						? t(
								"paidApprovalDescription",
								"Approving allows checkout. Access starts after payment. Only the owner can approve or grant complimentary access.",
							)
						: t(
								"onlyProjectAdminsCanReviewWhoAsksToJoin",
								"Only project admins can review who asks to join.",
							)
				}
				missing={[RolePermissions.Admin]}
				roleName={access.roleName}
			/>
		);
	}

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
							paid={paid}
							canOwn={access.canOwn}
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
	paid,
	canOwn,
}: Readonly<{
	appId: string;
	request: IJoinRequest;
	refresh: () => void;
	paid: boolean;
	canOwn: boolean;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const payments = usePayments();
	const [confirmComp, setConfirmComp] = useState(false);
	const [compBusy, setCompBusy] = useState(false);
	const user = useInvoke(backend.userState.lookupUser, backend.userState, [
		request.user_id,
	]);
	const userData = user.data;

	// A revoked role is not a network blip: surface what the server said instead
	// of inviting the admin to retry an action they no longer hold.
	const acceptRequest = useCallback(async () => {
		try {
			await backend.teamState.acceptJoinRequest(appId, request.id);
			refresh();
		} catch (error) {
			console.error("Failed to accept request:", error);
			toast.error(
				apiErrorMessage(
					error,
					t("failedToAcceptRequest", "Failed to accept the request."),
				),
			);
		}
	}, [backend, appId, request.id, refresh, t]);

	const declineRequest = useCallback(async () => {
		try {
			await backend.teamState.rejectJoinRequest(appId, request.id);
			refresh();
		} catch (error) {
			console.error("Failed to decline request:", error);
			toast.error(
				apiErrorMessage(
					error,
					t("failedToDeclineRequest", "Failed to decline the request."),
				),
			);
		}
	}, [backend, appId, request.id, refresh, t]);

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
						{paid && request.approved_at
							? t("purchaseApproved", "Purchase approved")
							: t("wantsToJoin", "Wants to join")}
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
				<Button
					size="sm"
					onClick={acceptRequest}
					disabled={paid && (!canOwn || !!request.approved_at)}
				>
					<CheckIcon className="size-3.5" />
					{paid
						? t("approvePurchase", "Approve purchase")
						: t("approve", "Approve")}
				</Button>
				{paid && canOwn && (
					<Button
						size="sm"
						variant="outline"
						disabled={compBusy}
						onClick={async () => {
							if (!confirmComp) {
								setConfirmComp(true);
								return;
							}
							setCompBusy(true);
							try {
								await payments.request(
									`apps/${encodeURIComponent(appId)}/marketplace/comp/${encodeURIComponent(request.user_id)}`,
									"POST",
									{ confirm: true },
								);
								refresh();
							} catch (error) {
								toast.error(
									apiErrorMessage(
										error,
										t(
											"compFailed",
											"Complimentary access could not be granted.",
										),
									),
								);
							} finally {
								setCompBusy(false);
							}
						}}
					>
						{confirmComp
							? t("confirmComp", "Confirm free access")
							: t("grantComp", "Grant complimentary access")}
					</Button>
				)}
				<Button size="sm" variant="outline" onClick={declineRequest}>
					<XIcon className="size-3.5" />
					{t("decline", "Decline")}
				</Button>
			</TeamRowActions>
		</div>
	);
}
