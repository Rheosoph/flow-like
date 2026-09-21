"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { isMissingResourceError } from "../../../lib/api-error";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { useBackend } from "../../../state/backend-state";
import type {
	IAuditSecret,
	IAuditWebhook,
	IAuditWebhookInput,
	IAuditWebhookSaved,
} from "../../audit/types";

function webhookPath(appId: string, suffix = ""): string {
	return `apps/${encodeURIComponent(appId)}/audit/webhook${suffix}`;
}

export function useAuditWebhook(profile: IProfile | undefined, appId: string) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const queryKey = useMemo(
		() => ["audit", "webhook", profile?.hub, appId],
		[profile?.hub, appId],
	);

	const webhook = useQuery({
		queryKey,
		queryFn: async (): Promise<IAuditWebhook | null> => {
			if (!profile) throw new Error("Profile not loaded");
			try {
				return await backend.apiState.get<IAuditWebhook>(
					profile,
					webhookPath(appId),
				);
			} catch (error) {
				if (isMissingResourceError(error)) return null;
				throw error;
			}
		},
		enabled: !!profile && appId.length > 0,
		meta: { persist: false },
	});

	const save = useMutation({
		mutationFn: (input: IAuditWebhookInput) => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.put<IAuditWebhookSaved>(
				profile,
				webhookPath(appId),
				input,
			);
		},
		onSuccess: ({ secret: _secret, ...view }) => {
			queryClient.setQueryData<IAuditWebhook | null>(queryKey, view);
		},
	});

	const rotate = useMutation({
		mutationFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.post<IAuditSecret>(
				profile,
				webhookPath(appId, "/rotate"),
			);
		},
	});

	const remove = useMutation({
		mutationFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.del<void>(profile, webhookPath(appId));
		},
		onSuccess: () => {
			queryClient.setQueryData<IAuditWebhook | null>(queryKey, null);
		},
	});

	return { webhook, save, rotate, remove };
}
