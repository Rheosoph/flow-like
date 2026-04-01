"use client";

import { useQuery } from "@tanstack/react-query";
import { useBackend, useInvoke } from "@tm9657/flow-like-ui";
import { AppPublicationPage } from "@tm9657/flow-like-ui/components/settings/visibility-status/app-publication-page";
import {
	type AppPublicationRequestItem,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@tm9657/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { useRouter, useSearchParams } from "next/navigation";

export default function Page() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const router = useRouter();
	const id = searchParams.get("id");

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
		queryKey: ["app-publication-requests", id],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawAppPublicationRequestItem[]>(
				profile.data.hub_profile,
				`apps/${id}/publication`,
			);
		},
		enabled: !!profile.data && !!id,
		select: normalizeAppPublicationRequests,
	});

	return (
		<AppPublicationPage
			requests={publicationRequests.data ?? []}
			isLoading={publicationRequests.isLoading}
			error={
				publicationRequests.isError
					? (publicationRequests.error?.message ??
						"Failed to load publication review history")
					: null
			}
			onBack={() => router.push(`/library/config?id=${id}`)}
		/>
	);
}
