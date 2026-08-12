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
import { ForkAppCard } from "@flow-like/flow-like-ui/components/settings/forking/fork-app-card";
import { AppAiActWizard } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-ai-act-wizard";
import {
	type AppPublicationRequestItem,
	AppPublicationReviewCard,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityStatusSwitcher as SharedVisibilityStatusSwitcher } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-status-switcher";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

interface SectionProps {
	localApp: IApp;
	appName: string;
	refreshApp: () => void;
	canEdit: boolean;
}

/**
 * Who can reach the app: visibility, forking and the fork entry point.
 * Mounted inside the dashboard's Access inspector panel.
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
			<AllowForkingCard
				localApp={localApp}
				canEdit={canEdit}
				onChanged={refreshApp}
			/>
			<ForkAppCard appId={localApp.id} appName={appName} target="online" />
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
	const backend = useBackend();
	const features = useFeatures();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const publicationRequests = useQuery<
		RawAppPublicationRequestItem[],
		Error,
		AppPublicationRequestItem[]
	>({
		queryKey: ["app-publication-requests", localApp.id],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawAppPublicationRequestItem[]>(
				profile.data.hub_profile,
				`apps/${localApp.id}/publication`,
			);
		},
		enabled: !!profile.data && canEdit,
		select: normalizeAppPublicationRequests,
	});

	const reviewError = publicationRequests.isError
		? (publicationRequests.error?.message ??
			"Failed to load publication review history")
		: null;

	return (
		<>
			{localApp.visibility !== IAppVisibility.Offline &&
				features.data?.ai_act &&
				canEdit && <AppAiActWizard appId={localApp.id} />}
			<AppPublicationReviewCard
				requests={publicationRequests.data ?? []}
				isLoading={publicationRequests.isLoading}
				error={reviewError}
			/>
		</>
	);
}
