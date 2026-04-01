"use client";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { StorageSystem } from "@tm9657/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";

export default function Page() {
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const prefix = searchParams.get("prefix") ?? "";
	const router = useRouter();

	const resolveFullPath = useCallback(
		(location: string) =>
			invoke<string>("storage_to_fullpath", {
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

	return (
		<StorageSystem
			appId={id ?? ""}
			prefix={decodeURIComponent(prefix)}
			fileToUrl={fileToUrl}
			revealInExplorer={handleRevealInExplorer}
			openWithApp={handleOpenWithApp}
			listAppsForFile={handleListAppsForFile}
			updatePrefix={(prefix) => {
				router.push(
					`/library/config/storage?id=${id}&prefix=${encodeURIComponent(prefix)}`,
				);
			}}
			key={`${id}-${prefix}`}
		/>
	);
}
