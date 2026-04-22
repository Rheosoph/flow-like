"use client";

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { StorageSystem, useBackend } from "@tm9657/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";

export default function Page() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const prefix = searchParams.get("prefix") ?? "";
	const router = useRouter();

	const resolveFullPath = useCallback(
		(location: string) =>
			invoke<string>("storage_user_to_fullpath", {
				appId: id,
				prefix: location,
			}),
		[id],
	);

	const fileToUrl = useCallback(
		async (file: string) => convertFileSrc(await resolveFullPath(file)),
		[resolveFullPath],
	);

	const handleRevealInExplorer = useCallback(
		async (location: string) => {
			const fullPath = await resolveFullPath(location);
			await revealItemInDir(fullPath);
		},
		[resolveFullPath],
	);

	const handleOpenWithApp = useCallback(
		async (location: string, appPath?: string) => {
			const fullPath = await resolveFullPath(location);
			if (!appPath) {
				await openPath(fullPath);
				return;
			}
			await invoke("open_file_with_app", { filePath: fullPath, appPath });
		},
		[resolveFullPath],
	);

	const handleListAppsForFile = useCallback(
		async (location: string) => {
			const fullPath = await resolveFullPath(location);
			return invoke<{ name: string; path: string; is_default: boolean }[]>(
				"list_apps_for_file",
				{ filePath: fullPath },
			);
		},
		[resolveFullPath],
	);

	const operations = useMemo(
		() => ({
			listStorageItems: (appId: string, targetPrefix: string) =>
				backend.storageState.listStorageItemsUser(appId, targetPrefix),
			deleteStorageItems: (appId: string, prefixes: string[]) =>
				backend.storageState.deleteStorageItemsUser(appId, prefixes),
			downloadStorageItems: (appId: string, prefixes: string[]) =>
				backend.storageState.downloadStorageItemsUser(appId, prefixes),
			uploadStorageItems: (
				appId: string,
				targetPrefix: string,
				files: File[],
				onProgress?: (progress: number) => void,
			) =>
				backend.storageState.uploadStorageItemsUser(
					appId,
					targetPrefix,
					files,
					onProgress,
				),
			writeStorageItems: backend.storageState.writeStorageItems?.bind(
				backend.storageState,
			),
		}),
		[backend.storageState],
	);

	return (
		<StorageSystem
			appId={id ?? ""}
			prefix={decodeURIComponent(prefix)}
			fileToUrl={fileToUrl}
			title="User Storage"
			storageScopeKey="user"
			operations={operations}
			revealInExplorer={handleRevealInExplorer}
			openWithApp={handleOpenWithApp}
			listAppsForFile={handleListAppsForFile}
			updatePrefix={(nextPrefix) => {
				router.push(
					`/library/config/user-storage?id=${id}&prefix=${encodeURIComponent(nextPrefix)}`,
				);
			}}
			key={`${id}-${prefix}`}
		/>
	);
}
