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
 * surrounding acquisition flow. Callers are responsible for invalidating the
 * `getSettingsProfile`/`getApps` queries afterwards (e.g. via `useInvalidateInvoke`).
 */
export async function addAppToProfile(
	backend: IBackendState,
	appId: string,
): Promise<void> {
	try {
		const profile = await backend.userState.getSettingsProfile();
		await backend.userState.updateProfileApp(
			profile,
			{ app_id: appId, favorite: false, pinned: false },
			"Upsert",
		);
	} catch (error) {
		console.error("Failed to add app to profile:", error);
	}
}
