import { useState } from "react";
import { ProfileSettingsPage } from "../../packages/ui/components/settings/profile/profile-settings-page";
import { useProfileDraft } from "../../packages/ui/components/settings/profile/use-profile-draft";
import {
	IConnectionMode,
	type ISettingsProfile,
	IThemes,
} from "../../packages/ui/types";

export function WorkspaceFixture() {
	const [source, setSource] = useState<ISettingsProfile>({
		hub_profile: {
			id: "workspace-fixture",
			name: "Research workspace",
			description: "My apps, models, and preferences for research projects.",
			hub: "fixture.invalid",
			hubs: [],
			bits: [],
			apps: [],
			tags: ["Research"],
			interests: ["Data analysis"],
			settings: { connection_mode: IConnectionMode.Simplebezier },
			secure: true,
			created: "2026-09-05",
			updated: "2026-09-05",
		},
		execution_settings: { gpu_mode: true, max_context_size: 8192 },
		created: "2026-09-05",
		updated: "2026-09-05",
	});
	const [state] = useState(() => ({
		saves: [] as ISettingsProfile[],
		failSave: false,
		deleted: false,
	}));
	const draft = useProfileDraft(source, async (profile) => {
		if (state.failSave)
			throw new Error("The connection failed. Please try again.");
		state.saves.push(structuredClone(profile));
		setSource(structuredClone(profile));
	});
	Object.assign(window, { workspaceQa: state });
	if (!draft.profile) return null;
	if (state.deleted) return <p>Profile removed from this device.</p>;
	return (
		<ProfileSettingsPage
			profile={draft.profile}
			isCustomTheme={Boolean(draft.profile.hub_profile.theme)}
			hasChanges={draft.status !== "saved"}
			saveStatus={draft.status}
			saveError={draft.error}
			onRetrySave={draft.retry}
			onProfileUpdate={draft.update}
			themeTranslation={
				{ [IThemes.FLOW_LIKE]: undefined } as Record<IThemes, unknown>
			}
			supportsExecutionSettings={
				!new URLSearchParams(location.search).has("web")
			}
			canDeleteProfile
			deleteScope="local"
			onProfileImageChange={async () => {
				throw new Error(
					"The image upload failed. Your previous image is still in use.",
				);
			}}
			onProfileDelete={async () => {
				state.deleted = true;
			}}
		/>
	);
}
