"use client";

import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import {
	type ISettingsProfile,
	IThemes,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { ProfileSettingsPage } from "@tm9657/flow-like-ui/components/settings/profile/profile-settings-page";
import { useDebounce } from "@uidotdev/usehooks";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useTauriInvoke } from "../../../components/useInvoke";
import AMBER_MINIMAL from "./themes/amber-minimal.json";
import AMETHYST_HAZE from "./themes/amethyst-haze.json";
import BOLD_TECH from "./themes/bold-tech.json";
import BUBBLEGUM from "./themes/bubblegum.json";
import CAFFEINE from "./themes/caffeine.json";
import CANDYLAND from "./themes/candyland.json";
import CATPPUCHIN from "./themes/catppuccin.json";
import CLAYMORPHISM from "./themes/claymorphism.json";
import CLEAN_SLATE from "./themes/clean-slate.json";
import COSMIC_NIGHT from "./themes/cosmic-night.json";
import CYBER_PUNK from "./themes/cyber-punk.json";
import DOOM from "./themes/doom.json";
import GRAPHITE from "./themes/graphite.json";
import KODAMA_GROVE from "./themes/kodama-grove.json";
import LUXURY from "./themes/luxury.json";
import MIDNIGHT_BLOOM from "./themes/midnight-bloom.json";
import MOCHA_MOUSSE from "./themes/mocha-mousse.json";
import MODERN_MINIMAL from "./themes/modern-minimal.json";
import MONO from "./themes/mono.json";
import NATURE from "./themes/nature.json";
import NEO_BRUTALISM from "./themes/neo-brutalism.json";
import NORTHERN_LIGHTS from "./themes/northern-lights.json";
import NOTEBOOK from "./themes/notebook.json";
import OCEAN_BREEZE from "./themes/ocean-breeze.json";
import PASTEL_DREAMS from "./themes/pastel-dreams.json";
import PERPETUITY from "./themes/perpetuity.json";
import QUANTUM_ROSE from "./themes/quantum-rose.json";
import RETRO_ARCADE from "./themes/retro-arcade.json";
import SOFT_POP from "./themes/soft-pop.json";
import SOLAR_DUSK from "./themes/solar-dusk.json";
import STARRY_NIGHT from "./themes/starry-night.json";
import SUNSET_HORIZON from "./themes/sunset-horizon.json";
import TANGERINE from "./themes/tangerine.json";
import VINTAGE_PAPER from "./themes/vintage-paper.json";
import VIOLET_BLOOM from "./themes/violet-bloom.json";

const THEME_TRANSLATION: Record<IThemes, unknown> = {
	[IThemes.FLOW_LIKE]: undefined,
	[IThemes.AMBER_MINIMAL]: AMBER_MINIMAL,
	[IThemes.AMETHYST_HAZE]: AMETHYST_HAZE,
	[IThemes.BOLD_TECH]: BOLD_TECH,
	[IThemes.BUBBLEGUM]: BUBBLEGUM,
	[IThemes.CAFFEINE]: CAFFEINE,
	[IThemes.CANDYLAND]: CANDYLAND,
	[IThemes.CATPPUCCIN]: CATPPUCHIN,
	[IThemes.CLAYMORPHISM]: CLAYMORPHISM,
	[IThemes.CLEAN_SLATE]: CLEAN_SLATE,
	[IThemes.COSMIC_NIGHT]: COSMIC_NIGHT,
	[IThemes.CYBERPUNK]: CYBER_PUNK,
	[IThemes.DOOM_64]: DOOM,
	[IThemes.ELEGANT_LUXURY]: LUXURY,
	[IThemes.GRAPHITE]: GRAPHITE,
	[IThemes.KODAMA_GROVE]: KODAMA_GROVE,
	[IThemes.MIDNIGHT_BLOOM]: MIDNIGHT_BLOOM,
	[IThemes.MOCHA_MOUSSE]: MOCHA_MOUSSE,
	[IThemes.MODERN_MINIMAL]: MODERN_MINIMAL,
	[IThemes.MONO]: MONO,
	[IThemes.NATURE]: NATURE,
	[IThemes.NEO_BRUTALISM]: NEO_BRUTALISM,
	[IThemes.NORTHERN_LIGHTS]: NORTHERN_LIGHTS,
	[IThemes.NOTEBOOK]: NOTEBOOK,
	[IThemes.OCEAN_BREEZE]: OCEAN_BREEZE,
	[IThemes.PASTEL_DREAMS]: PASTEL_DREAMS,
	[IThemes.PERPETUITY]: PERPETUITY,
	[IThemes.QUANTUM_ROSE]: QUANTUM_ROSE,
	[IThemes.RETRO_ARCADE]: RETRO_ARCADE,
	[IThemes.SOLAR_DUSK]: SOLAR_DUSK,
	[IThemes.STARRY_NIGHT]: STARRY_NIGHT,
	[IThemes.SUNSET_HORIZON]: SUNSET_HORIZON,
	[IThemes.SOFT_POP]: SOFT_POP,
	[IThemes.TANGERINE]: TANGERINE,
	[IThemes.VIOLET_BLOOM]: VIOLET_BLOOM,
	[IThemes.VINTAGE_PAPER]: VINTAGE_PAPER,
};

export default function SettingsProfilesPage() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const auth = useAuth();
	const router = useRouter();
	const profiles = useTauriInvoke<Record<string, ISettingsProfile>>(
		"get_profiles",
		{},
	);

	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [localProfile, setLocalProfile] = useState<ISettingsProfile | null>(
		null,
	);
	const debouncedLocalProfile = useDebounce(localProfile, 500);
	const [hasChanges, setHasChanges] = useState(false);
	const isSavingRef = useRef(false);
	const localProfileRef = useRef(localProfile);
	localProfileRef.current = localProfile;

	useEffect(() => {
		if (currentProfile.data && !isSavingRef.current) {
			setLocalProfile(currentProfile.data);
			setHasChanges(false);
		}
	}, [currentProfile.data]);

	const isCustomTheme = useMemo(() => {
		const id = localProfile?.hub_profile?.theme?.id;
		return !!id && !Object.values(IThemes).includes(id as IThemes);
	}, [localProfile]);

	const upsertProfile = useCallback(
		async (profile: ISettingsProfile) => {
			isSavingRef.current = true;
			try {
				await invoke("upsert_profile", { profile });
				await profiles.refetch();
				await invalidate(backend.userState.getProfile, []);
				await currentProfile.refetch();
			} finally {
				isSavingRef.current = false;
			}
		},
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[invalidate],
	);

	useEffect(() => {
		if (debouncedLocalProfile && hasChanges) {
			upsertProfile(debouncedLocalProfile);
			setHasChanges(false);
		}
	}, [debouncedLocalProfile, hasChanges, upsertProfile]);

	const updateProfile = useCallback((updates: Partial<ISettingsProfile>) => {
		const current = localProfileRef.current;
		if (!current) return;
		const now = new Date().toISOString();
		const hubProfile = updates.hub_profile
			? { ...current.hub_profile, ...updates.hub_profile, updated: now }
			: { ...current.hub_profile, updated: now };
		const newProfile = {
			...current,
			...updates,
			hub_profile: hubProfile,
			updated: now,
		};
		setLocalProfile(newProfile);
		setHasChanges(true);
	}, []);

	const handleProfileImageChange = useCallback(async () => {
		const current = localProfileRef.current;
		if (!current) return;
		await invoke("change_profile_image", { profile: current });
		await profiles.refetch();
		await invalidate(backend.userState.getProfile, []);
		await currentProfile.refetch();
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [invalidate]);

	const profileCount = Object.keys(profiles.data ?? {}).length;

	const handleProfileDelete = useCallback(async () => {
		const current = localProfileRef.current;
		if (!current?.hub_profile.id) return;
		if (profileCount <= 1) return;

		const profileId = current.hub_profile.id;

		// Delete from server if authenticated
		if (auth.isAuthenticated && auth.user?.access_token) {
			try {
				const profile = await invoke<{ hub?: string; secure?: boolean }>(
					"get_current_profile",
				).catch(() => null);
				const hubUrl = profile?.hub;
				const baseUrl =
					process.env.NEXT_PUBLIC_API_URL ?? hubUrl ?? "api.flow-like.com";
				const protocol = profile?.secure === false ? "http" : "https";
				const apiBase = (
					baseUrl.startsWith("http") ? baseUrl : `${protocol}://${baseUrl}`
				).replace(/\/+$/, "");

				const response = await tauriFetch(
					`${apiBase}/api/v1/profile/${encodeURIComponent(profileId)}`,
					{
						method: "DELETE",
						headers: {
							Authorization: `Bearer ${auth.user.access_token}`,
						},
					},
				);
				if (!response.ok && response.status !== 404) {
					console.warn(
						"[ProfileDelete] Server delete failed:",
						response.status,
					);
				}
			} catch (err) {
				console.warn("[ProfileDelete] Server delete error:", err);
			}
		}

		// Switch to another profile before deleting
		const allProfiles = profiles.data ?? {};
		const otherProfileId = Object.keys(allProfiles).find(
			(id) => id !== profileId,
		);
		if (otherProfileId) {
			await invoke("set_current_profile", { profileId: otherProfileId });
		}

		// Delete locally
		await invoke("delete_profile", { profileId });

		toast.success("Profile deleted");

		await profiles.refetch();
		await invalidate(backend.userState.getProfile, []);
		await invalidate(backend.userState.getSettingsProfile, []);
		await currentProfile.refetch();
		router.push("/");
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [profileCount, auth, invalidate, router]);

	if (!localProfile) {
		return (
			<main className="flex flex-col items-center justify-center w-full flex-1 min-h-0 py-12">
				<div className="animate-spin rounded-full h-32 w-32 border-b-2 border-primary" />
			</main>
		);
	}

	return (
		<ProfileSettingsPage
			profile={localProfile}
			isCustomTheme={isCustomTheme}
			hasChanges={hasChanges}
			themeTranslation={THEME_TRANSLATION}
			onProfileUpdate={updateProfile}
			onProfileImageChange={handleProfileImageChange}
			onProfileDelete={handleProfileDelete}
			canDeleteProfile={profileCount > 1}
		/>
	);
}
