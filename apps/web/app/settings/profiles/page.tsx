"use client";

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

type UpsertProfileResponse = {
	icon_upload_url?: string | null;
	thumbnail_upload_url?: string | null;
};

const IMAGE_MIME_TO_EXT: Record<string, string> = {
	"image/jpeg": "jpg",
	"image/png": "png",
	"image/webp": "webp",
	"image/gif": "gif",
	"image/svg+xml": "svg",
};

function getImageExtension(file: File): string | null {
	const fromName = file.name.split(".").pop()?.toLowerCase();
	if (fromName) {
		if (fromName === "jpeg") return "jpg";
		return fromName;
	}
	return IMAGE_MIME_TO_EXT[file.type] ?? null;
}

function pickImageFile(): Promise<File | null> {
	return new Promise((resolve) => {
		const input = document.createElement("input");
		let resolved = false;
		const finish = (file: File | null) => {
			if (resolved) return;
			resolved = true;
			window.removeEventListener("focus", onFocus, true);
			resolve(file);
		};
		const onFocus = () => {
			window.setTimeout(() => {
				finish(input.files?.[0] ?? null);
			}, 0);
		};

		input.type = "file";
		input.accept = "image/*";
		input.onchange = () => finish(input.files?.[0] ?? null);
		window.addEventListener("focus", onFocus, true);
		input.click();
	});
}

async function uploadToSignedUrl(url: string, file: File): Promise<void> {
	const headers: HeadersInit = {
		"Content-Type": file.type || "application/octet-stream",
	};

	if (url.includes(".blob.core.windows.net")) {
		headers["x-ms-blob-type"] = "BlockBlob";
	}

	const response = await fetch(url, {
		method: "PUT",
		body: file,
		headers,
	});

	if (!response.ok) {
		throw new Error(`Upload failed: ${response.status} ${response.statusText}`);
	}
}

export default function SettingsProfilesPage() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const auth = useAuth();
	const router = useRouter();

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

	const updateProfile = useCallback((updates: Partial<ISettingsProfile>) => {
		const current = localProfileRef.current;
		if (!current) return;
		const newProfile = { ...current, ...updates };
		setLocalProfile(newProfile);
		setHasChanges(true);
	}, []);

	const requestProfileUpsert = useCallback(
		async (
			profileId: string,
			body: Record<string, unknown>,
		): Promise<UpsertProfileResponse> => {
			if (!auth.user?.access_token) {
				throw new Error("Missing access token");
			}
			const baseUrl =
				process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";
			const response = await fetch(`${baseUrl}/api/v1/profile/${profileId}`, {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
					Authorization: `Bearer ${auth.user.access_token}`,
				},
				body: JSON.stringify(body),
			});
			if (!response.ok) {
				const message = await response.text().catch(() => "");
				throw new Error(message || `Failed with status ${response.status}`);
			}
			return (await response.json()) as UpsertProfileResponse;
		},
		[auth.user?.access_token],
	);

	const upsertProfile = useCallback(
		async (profile: ISettingsProfile) => {
			if (!profile.hub_profile.id) return;
			isSavingRef.current = true;
			try {
				await requestProfileUpsert(profile.hub_profile.id, {
					name: profile.hub_profile.name,
					description: profile.hub_profile.description,
					interests: profile.hub_profile.interests,
					tags: profile.hub_profile.tags,
					theme: profile.hub_profile.theme,
					bit_ids: profile.hub_profile.bits,
					apps: profile.hub_profile.apps,
					hub: profile.hub_profile.hub,
					hubs: profile.hub_profile.hubs,
					settings: profile.execution_settings,
				});
				await invalidate(backend.userState.getProfile, []);
				await invalidate(backend.userState.getAllSettingsProfiles, []);
				await currentProfile.refetch();
			} finally {
				isSavingRef.current = false;
			}
		},
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[invalidate, requestProfileUpsert],
	);

	useEffect(() => {
		if (!debouncedLocalProfile || !hasChanges) return;
		void upsertProfile(debouncedLocalProfile)
			.then(() => setHasChanges(false))
			.catch((error) => {
				console.error("Failed to save profile changes:", error);
			});
	}, [debouncedLocalProfile, hasChanges, upsertProfile]);

	const handleProfileImageChange = useCallback(async () => {
		const current = localProfileRef.current;
		if (!current?.hub_profile.id) return;

		const file = await pickImageFile();
		if (!file) return;

		const extension = getImageExtension(file);
		if (!extension) {
			toast.error("Unsupported image format");
			return;
		}

		try {
			const result = await requestProfileUpsert(current.hub_profile.id, {
				icon_upload_ext: extension,
			});

			if (!result.icon_upload_url) {
				throw new Error("No upload URL returned");
			}

			await uploadToSignedUrl(result.icon_upload_url, file);

			await invalidate(backend.userState.getProfile, []);
			await invalidate(backend.userState.getAllSettingsProfiles, []);
			await currentProfile.refetch();
			toast.success("Profile image updated");
		} catch (error) {
			console.error("Failed to update profile image:", error);
			toast.error("Failed to update profile image");
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [invalidate, requestProfileUpsert]);

	const allProfiles = useInvoke(
		backend.userState.getAllSettingsProfiles,
		backend.userState,
		[],
	);

	const profileCount = allProfiles.data?.length ?? 1;

	const handleProfileDelete = useCallback(async () => {
		const current = localProfileRef.current;
		if (!current?.hub_profile.id) return;
		if (!auth.user?.access_token) return;
		if (profileCount <= 1) return;

		const profileId = current.hub_profile.id;
		const baseUrl =
			process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";

		const response = await fetch(`${baseUrl}/api/v1/profile/${profileId}`, {
			method: "DELETE",
			headers: {
				Authorization: `Bearer ${auth.user.access_token}`,
			},
		});

		if (!response.ok && response.status !== 404) {
			throw new Error(`Failed to delete profile: ${response.status}`);
		}

		toast.success("Profile deleted");
		await invalidate(backend.userState.getProfile, []);
		await invalidate(backend.userState.getSettingsProfile, []);
		await invalidate(backend.userState.getAllSettingsProfiles, []);
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
