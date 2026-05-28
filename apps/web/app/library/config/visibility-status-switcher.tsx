"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	type IApp,
	type IAppVisibility,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import {
	type AppPublicationRequestItem,
	AppPublicationReviewCard,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityStatusSwitcher as SharedVisibilityStatusSwitcher } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-status-switcher";
import { useCallback } from "react";

interface VisibilityStatusSwitcherProps {
	localApp: IApp;
	refreshApp: () => void;
	canEdit: boolean;
}

export function VisibilityStatusSwitcher({
	localApp,
	refreshApp,
	canEdit,
}: Readonly<VisibilityStatusSwitcherProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const queryClient = useQueryClient();
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
			<AppPublicationReviewCard
				requests={publicationRequests.data ?? []}
				isLoading={publicationRequests.isLoading}
				error={
					publicationRequests.isError
						? (publicationRequests.error?.message ??
							"Failed to load publication review history")
						: null
				}
			/>
		</>
	);
}
