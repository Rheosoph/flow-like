import type { QueryClient } from "@tanstack/react-query";
import type { IBackendState } from "../state/backend-state";

/**
 * Register an app in the user's active profile so it appears in the library.
 *
 * Membership alone does not surface an app: the library default view only lists
 * apps present in the current profile's `apps` list. Every path that grants a
 * user access to an app (store "Get"/purchase, invite accept, join link) must
 * call this, otherwise the app stays "member but hidden".
 *
 * Failures are swallowed (logged only) so a profile-sync hiccup never aborts the
 * surrounding acquisition flow. Pass a `queryClient` to refresh the library.
 */
export async function addAppToProfile(
	backend: IBackendState,
	appId: string,
	queryClient?: QueryClient,
): Promise<void> {
	try {
		const profile = await backend.userState.getSettingsProfile();
		await backend.userState.updateProfileApp(
			profile,
			{ app_id: appId, favorite: false, pinned: false },
			"Upsert",
		);
		queryClient?.invalidateQueries({ queryKey: ["getSettingsProfile"] });
		queryClient?.invalidateQueries({ queryKey: ["getApps"] });
	} catch (error) {
		console.error("Failed to add app to profile:", error);
	}
}
