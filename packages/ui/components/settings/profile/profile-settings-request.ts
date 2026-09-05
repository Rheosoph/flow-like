import { IConnectionMode, type ISettingsProfile } from "../../../types";

/** Only fields edited here are sent; app membership and device preferences have separate owners. */
export function profileSettingsPatch(profile: ISettingsProfile) {
	return {
		name: profile.hub_profile.name.trim(),
		description: profile.hub_profile.description,
		interests: profile.hub_profile.interests,
		tags: profile.hub_profile.tags,
		theme: profile.hub_profile.theme ?? null,
		settings: profile.hub_profile.settings ?? {
			connection_mode: IConnectionMode.Simplebezier,
		},
	};
}
