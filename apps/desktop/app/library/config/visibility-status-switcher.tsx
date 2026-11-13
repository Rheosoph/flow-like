"use client";

import {
	type IApp,
	IAppVisibility,
	PermissionNotice,
	RolePermissions,
	useAppPermissions,
	useBackend,
	useFeatures,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { BotAccessCard } from "@flow-like/flow-like-ui/components/settings/bot-access/bot-access-card";
import { AllowForkingCard } from "@flow-like/flow-like-ui/components/settings/forking/allow-forking-card";
import { AppAiActWizard } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-ai-act-wizard";
import {
	type AppPublicationRequestItem,
	AppPublicationReviewCard,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityStatusSwitcher as SharedVisibilityStatusSwitcher } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-status-switcher";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { ForkAppButton } from "./fork-app-button";

interface SectionProps {
	localApp: IApp;
	appName: string;
	refreshApp: () => void;
	canEdit: boolean;
}

function usePublicationRequests(appId: string, enabled: boolean) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	return useQuery<
		RawAppPublicationRequestItem[],
		Error,
		AppPublicationRequestItem[]
	>({
		queryKey: ["app-publication-requests", appId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawAppPublicationRequestItem[]>(
				profile.data.hub_profile,
				`apps/${appId}/publication`,
			);
		},
		enabled: !!profile.data && enabled,
		select: normalizeAppPublicationRequests,
	});
}

/**
 * Everything that decides who can reach the app: visibility, forking and the
 * fork entry point. Mounted inside the dashboard's Access inspector panel.
 *
 * Both cards resolve the caller's own role themselves — visibility and
 * forking are `Owner` routes, and this mount site cannot know the reader.
 */
export function AppAccessSection({
	localApp,
	appName,
	refreshApp,
	canEdit,
}: Readonly<SectionProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const queryClient = useQueryClient();
	const isOffline = localApp.visibility === IAppVisibility.Offline;

	const handleVisibilityChange = useCallback(
		async (appId: string, newVisibility: IAppVisibility) => {
			await backend.appState.changeAppVisibility(appId, newVisibility);
			await invalidate(backend.appState.getApp, [appId]);
			await invalidate(backend.appState.getApps, []);
			await queryClient.invalidateQueries({
				queryKey: ["app-publication-requests", appId],
			});
			refreshApp();
		},
		[backend.appState, invalidate, queryClient, refreshApp],
	);

	return (
		<>
			<SharedVisibilityStatusSwitcher
				localApp={localApp}
				canEdit={canEdit}
				onVisibilityChange={handleVisibilityChange}
			/>
			{!isOffline && (
				<AllowForkingCard
					localApp={localApp}
					canEdit={canEdit}
					onChanged={refreshApp}
				/>
			)}
			<BotAccessCard
				localApp={localApp}
				canEdit={canEdit}
				onChanged={refreshApp}
			/>
			<ForkAppButton localApp={localApp} appName={appName} />
		</>
	);
}

/**
 * Conformity assessment and publication review history. Mounted inside the
 * dashboard's Compliance inspector panel.
 */
export function AppComplianceSection({
	localApp,
	canEdit,
}: Readonly<Omit<SectionProps, "refreshApp" | "appName">>) {
	const { t } = useTranslation("settings");
	const features = useFeatures();
	const permissions = useAppPermissions(localApp.id);
	const isOffline = localApp.visibility === IAppVisibility.Offline;
	/** `GET /apps/{id}/publication` is `ensure_permission!(.., Admin)`. */
	const canReadReview = permissions.can(RolePermissions.Admin);
	/** Every `apps/{id}/ai-act/*` route is `ensure_permission!(.., Owner)`. */
	const canAssess = permissions.can(RolePermissions.Owner);
	const publicationRequests = usePublicationRequests(
		localApp.id,
		canEdit && !isOffline && canReadReview,
	);

	if (isOffline) {
		return (
			<p className="text-sm text-muted-foreground">
				{i18next.t(
					"offlineProjectsAreNeverListedSoNoConformityAssessmentOrPublicationReviewIsRequiredBringTheProjectOnlineToStart",
					"Offline projects are never listed, so no conformity assessment or publication review is required. Bring the project online to start.",
				)}
			</p>
		);
	}

	const reviewError = publicationRequests.isError
		? (publicationRequests.error?.message ??
			"Failed to load publication review history")
		: null;

	return (
		<>
			{features.data?.ai_act && canEdit && canAssess && (
				<AppAiActWizard appId={localApp.id} />
			)}
			{canReadReview ? (
				<AppPublicationReviewCard
					requests={publicationRequests.data ?? []}
					isLoading={publicationRequests.isLoading}
					error={reviewError}
				/>
			) : (
				// The review card renders nothing on an empty list, so without this
				// a denied read would leave the panel silently blank.
				<PermissionNotice
					title={t(
						"publicationReviewUnavailable",
						"Publication review unavailable",
					)}
					description={t(
						"yourRoleCannotSeeThisProjectsPublicationRequestsOrAuditorFeedback",
						"Your role cannot see this project's publication requests or auditor feedback.",
					)}
					missing={[RolePermissions.Admin]}
				/>
			)}
		</>
	);
}
