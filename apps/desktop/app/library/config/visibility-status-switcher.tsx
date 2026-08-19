"use client";

import {
	type IApp,
	IAppVisibility,
	useBackend,
	useFeatures,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { AllowForkingCard } from "@flow-like/flow-like-ui/components/settings/forking/allow-forking-card";
import { AppAiActWizard } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-ai-act-wizard";
import {
	type AppPublicationRequestItem,
	AppPublicationReviewCard,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityStatusSwitcher as SharedVisibilityStatusSwitcher } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-status-switcher";
import { i18n as i18next } from "@flow-like/locales";
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
	const features = useFeatures();
	const isOffline = localApp.visibility === IAppVisibility.Offline;
	const publicationRequests = usePublicationRequests(
		localApp.id,
		canEdit && !isOffline,
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
			{features.data?.ai_act && canEdit && (
				<AppAiActWizard appId={localApp.id} />
			)}
			<AppPublicationReviewCard
				requests={publicationRequests.data ?? []}
				isLoading={publicationRequests.isLoading}
				error={reviewError}
			/>
		</>
	);
}
