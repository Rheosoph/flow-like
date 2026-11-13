"use client";

import { useMutation } from "@tanstack/react-query";
import { useMemo } from "react";
import { useInvalidateInvoke, useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import { NO_HIDDEN_BOTS, withBotHidden } from "./bot-visibility-model";

/** The bots the current profile hides, by visibility key, and the profile's name. */
export function useHiddenBots(): {
	keys: ReadonlySet<string>;
	profileName?: string;
} {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const hidden = profile.data?.hub_profile.hidden_bots;
	const keys = useMemo(
		() => (hidden?.length ? new Set(hidden) : NO_HIDDEN_BOTS),
		[hidden],
	);
	return { keys, profileName: profile.data?.hub_profile.name };
}

/**
 * Hides a bot in the current profile or shows it again. Changes run one after another, each on the
 * profile as the backend holds it then, so two quick switches or another device's change both stay.
 */
export function useSetBotHidden() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	return useMutation({
		scope: { id: "profile-hidden-bots" },
		mutationFn: async ({ key, hidden }: { key: string; hidden: boolean }) => {
			const profile = await backend.userState.getSettingsProfile();
			const current = profile.hub_profile.hidden_bots ?? [];
			const next = withBotHidden(current, key, hidden);
			if (next.length === current.length) return;
			await backend.userState.updateProfileHiddenBots(profile, next);
		},
		onSettled: () =>
			Promise.all([
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfile, []),
			]),
	});
}
