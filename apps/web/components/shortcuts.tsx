"use client";
import {
	Shortcuts as ShortcutsUI,
	nowSystemTime,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { IBitTypes } from "@flow-like/flow-like-ui/lib/schema/hub/bit-search-query";
import { useTranslation } from "@flow-like/locales";
import { useLiveQuery } from "dexie-react-hooks";
import { usePathname, useRouter } from "next/navigation";
import { useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { appsDB } from "../lib/apps-db";

export function Shortcuts() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const auth = useAuth();
	const invalidate = useInvalidateInvoke();
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const bits = useInvoke(backend.bitState.searchBits, backend.bitState, [
		{
			bit_types: [IBitTypes.Embedding, IBitTypes.ImageEmbedding],
		},
	]);

	const shortcuts = useLiveQuery(async () => {
		if (!currentProfile.data?.hub_profile.id) return [];
		return await appsDB.shortcuts
			.where("profileId")
			.equals(currentProfile.data.hub_profile.id)
			.sortBy("order");
	}, [currentProfile.data?.hub_profile.id]);

	// Fetch metadata for all apps
	const appMetadata = useInvoke(backend.appState.getApps, backend.appState, []);

	const handleCreateProject = useCallback(
		async (projectName: string, isOnline: boolean) => {
			const meta = {
				name: projectName,
				description: t(
					"codingProjectProjectname",
					"Coding project: {{projectName}}",
					{ projectName },
				),
				tags: ["coding", "development"],
				use_case: "Development",
				created_at: nowSystemTime(),
				updated_at: nowSystemTime(),
				preview_media: [],
			};

			const filter = new Set(currentProfile.data?.hub_profile.bits ?? []);
			const allBits = bits.data?.filter((bit) => filter.has(bit.id)) ?? [];

			const app = await backend.appState.createApp(
				meta,
				allBits.map((bit) => bit.id),
				isOnline,
			);

			if (currentProfile.data) {
				await backend.userState.updateProfileApp(
					currentProfile.data,
					{
						app_id: app.id,
						favorite: false,
						pinned: false,
					},
					"Upsert",
				);
			}

			const boards = await backend.boardState.getBoardSummaries(app.id);
			const firstBoard = boards?.[0];

			if (firstBoard) {
				router.push(`/flow?id=${firstBoard.id}&app=${app.id}`);
			} else {
				router.push(`/library/config?id=${app.id}`);
			}

			await invalidate(backend.appState.getApps, []);
		},
		[
			currentProfile.data,
			backend.appState,
			backend.userState,
			backend.boardState,
			bits.data,
			invalidate,
			router,
		],
	);

	return (
		<ShortcutsUI
			db={appsDB}
			shortcuts={shortcuts}
			currentProfileId={currentProfile.data?.hub_profile.id}
			pathname={pathname}
			onNavigate={(path: string) => router.push(path)}
			backend={backend}
			appMetadata={appMetadata.data}
			getAppMetadataById={(appId: string, metadata: any) => {
				const appData = metadata.find((a: any) => a[0].id === appId);
				return appData?.[1] || null;
			}}
			getBoardsByAppId={async (backend: any, appId: string) => {
				return await backend.boardState.getBoardSummaries(appId);
			}}
			toast={toast}
			auth={auth}
			onCreateProject={handleCreateProject}
			onShortcutsChanged={async (_profileId, nextShortcuts) => {
				if (!currentProfile.data) return;
				await backend.userState.updateProfileShortcuts(
					currentProfile.data,
					nextShortcuts,
				);
			}}
			bits={bits.data}
		/>
	);
}
