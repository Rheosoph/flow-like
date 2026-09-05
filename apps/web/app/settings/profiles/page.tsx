"use client";

import {
	type ISettingsProfile,
	IThemes,
	isAzureBlobStorageUrl,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { workspaceProfileDraftScope } from "@flow-like/flow-like-ui/components/settings/profile/profile-draft";
import { ProfileSettingsLoadState } from "@flow-like/flow-like-ui/components/settings/profile/profile-settings-load-state";
import { ProfileSettingsPage } from "@flow-like/flow-like-ui/components/settings/profile/profile-settings-page";
import { profileSettingsPatch } from "@flow-like/flow-like-ui/components/settings/profile/profile-settings-request";
import { useProfileDraft } from "@flow-like/flow-like-ui/components/settings/profile/use-profile-draft";
import { apiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { completeMediaUpload } from "@flow-like/flow-like-ui/lib/profile-media-upload";
import { useRouter } from "next/navigation";
import { useCallback, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { appsDB } from "../../../lib/apps-db";
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
	icon_upload_id?: string | null;
	upload_pending?: boolean;
	thumbnail_upload_url?: string | null;
};

const IMAGE_MIME_TO_EXT: Record<string, string> = {
	"image/jpeg": "jpg",
	"image/png": "png",
	"image/webp": "webp",
};

function pickImageFile(): Promise<File | null> {
	return new Promise((resolve) => {
		const input = document.createElement("input");
		input.type = "file";
		input.accept = "image/png,image/jpeg,image/webp";
		input.hidden = true;
		const finish = (file: File | null) => {
			input.remove();
			resolve(file);
		};
		input.addEventListener("change", () => finish(input.files?.[0] ?? null), {
			once: true,
		});
		input.addEventListener("cancel", () => finish(null), { once: true });
		document.body.appendChild(input);
		input.click();
	});
}

async function uploadToSignedUrl(url: string, file: File): Promise<void> {
	const headers: HeadersInit = {
		"Content-Type": file.type || "application/octet-stream",
	};

	if (isAzureBlobStorageUrl(url)) {
		headers["x-ms-blob-type"] = "BlockBlob";
	}

	const response = await fetch(url, {
		method: "PUT",
		body: file,
		headers,
	});

	if (!response.ok) {
		throw new Error(
			"The image upload failed. Your previous image is still in use. Try again.",
		);
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

	const requestProfileUpsert = useCallback(
		async (
			profileId: string,
			body: Record<string, unknown>,
		): Promise<UpsertProfileResponse> => {
			if (!auth.user?.access_token) {
				throw new Error("Sign in again to save your profile.");
			}
			const baseUrl =
				process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";
			const response = await fetch(
				`${baseUrl.replace(/\/+$/, "")}/api/v1/profile/${encodeURIComponent(profileId)}`,
				{
					method: "POST",
					headers: {
						"Content-Type": "application/json",
						Authorization: `Bearer ${auth.user.access_token}`,
					},
					body: JSON.stringify(body),
				},
			);
			if (!response.ok) {
				const message = await response.text().catch(() => "");
				throw apiResponseError(response, message);
			}
			return (await response.json()) as UpsertProfileResponse;
		},
		[auth.user?.access_token],
	);

	const upsertProfile = useCallback(
		async (profile: ISettingsProfile) => {
			if (!profile.hub_profile.id) throw new Error("Profile ID is missing.");
			await requestProfileUpsert(
				profile.hub_profile.id,
				profileSettingsPatch(profile),
			);
			await Promise.all([
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.userState.getAllSettingsProfiles, []),
				currentProfile.refetch(),
			]);
		},
		[
			requestProfileUpsert,
			invalidate,
			backend.userState,
			currentProfile.refetch,
		],
	);
	const draft = useProfileDraft(
		currentProfile.data,
		upsertProfile,
		workspaceProfileDraftScope(
			"web",
			auth.user?.profile.sub,
			process.env.NEXT_PUBLIC_API_URL ??
				currentProfile.data?.hub_profile.hub ??
				auth.user?.profile.iss,
		),
	);
	const localProfile = draft.profile;
	const isCustomTheme = useMemo(() => {
		const id = localProfile?.hub_profile.theme?.id;
		return !!id && !Object.values(IThemes).includes(id as IThemes);
	}, [localProfile]);

	const handleProfileImageChange = useCallback(async () => {
		const current = localProfile;
		if (!current?.hub_profile.id) return;

		const profileId = current.hub_profile.id;
		const file = await pickImageFile();
		if (!file) return;

		const extension = IMAGE_MIME_TO_EXT[file.type];
		if (!extension) {
			throw new Error("Choose a PNG, JPEG or WebP image.");
		}

		if (!file.size || file.size > 10 * 1024 * 1024)
			throw new Error("Choose an image smaller than 10 MB.");

		try {
			await draft.flush();
			const result = await requestProfileUpsert(current.hub_profile.id, {
				icon_upload_ext: extension,
			});

			if (!result.icon_upload_url || !result.icon_upload_id) {
				throw new Error("No upload URL returned");
			}

			await uploadToSignedUrl(result.icon_upload_url, file);
			await completeMediaUpload(() =>
				requestProfileUpsert(profileId, {
					icon_upload_id: result.icon_upload_id,
				}),
			);

			await invalidate(backend.userState.getProfile, []);
			await invalidate(backend.userState.getAllSettingsProfiles, []);
			await currentProfile.refetch();
			toast.success("Profile image updated");
		} catch (error) {
			console.error("Failed to update profile image:", error);
			throw error;
		}
	}, [
		localProfile,
		draft,
		invalidate,
		requestProfileUpsert,
		currentProfile,
		backend.userState,
	]);

	const allProfiles = useInvoke(
		backend.userState.getAllSettingsProfiles,
		backend.userState,
		[],
	);

	const profileCount = allProfiles.data?.length ?? 1;

	const handleProfileDelete = useCallback(async () => {
		const current = localProfile;
		if (!current?.hub_profile.id) return;
		if (!auth.user?.access_token)
			throw new Error("Sign in again before deleting your profile.");
		if (profileCount <= 1) return;

		const profileId = current.hub_profile.id;
		const baseUrl =
			process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";

		await draft.flush();
		const response = await fetch(
			`${baseUrl.replace(/\/+$/, "")}/api/v1/profile/${encodeURIComponent(profileId)}`,
			{
				method: "DELETE",
				headers: { Authorization: `Bearer ${auth.user.access_token}` },
			},
		);
		if (!response.ok && response.status !== 404)
			throw apiResponseError(response, await response.text().catch(() => ""));
		draft.forget(profileId);

		if (typeof window !== "undefined") {
			const remainingProfile = allProfiles.data?.find(
				(profile) => profile.hub_profile.id !== profileId,
			);
			if (remainingProfile?.hub_profile.id) {
				localStorage.setItem(
					"flow-like-profile-id",
					remainingProfile.hub_profile.id,
				);
			} else {
				localStorage.removeItem("flow-like-profile-id");
			}
			localStorage.removeItem(`flow-like-offline-apps-${profileId}`);
		}
		await appsDB.shortcuts.where("profileId").equals(profileId).delete();

		toast.success("Profile deleted");
		await invalidate(backend.userState.getProfile, []);
		await invalidate(backend.userState.getSettingsProfile, []);
		await invalidate(backend.userState.getAllSettingsProfiles, []);
		await currentProfile.refetch();
		router.push("/");
	}, [
		localProfile,
		draft,
		currentProfile,
		backend.userState,
		profileCount,
		auth,
		invalidate,
		router,
		allProfiles.data,
	]);

	if (!localProfile) {
		return (
			<ProfileSettingsLoadState
				error={currentProfile.error}
				onRetry={() => void currentProfile.refetch()}
			/>
		);
	}

	return (
		<ProfileSettingsPage
			key={localProfile.hub_profile.id}
			profile={localProfile}
			isCustomTheme={isCustomTheme}
			hasChanges={draft.status !== "saved"}
			saveStatus={draft.status}
			saveError={draft.error}
			onRetrySave={draft.retry}
			supportsExecutionSettings={false}
			themeTranslation={THEME_TRANSLATION}
			onProfileUpdate={draft.update}
			onProfileImageChange={handleProfileImageChange}
			onProfileDelete={handleProfileDelete}
			canDeleteProfile={profileCount > 1}
		/>
	);
}
