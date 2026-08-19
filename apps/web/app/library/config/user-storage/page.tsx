"use client";

import { useTranslation } from "@flow-like/locales";
import { StorageSystem, useBackend } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";

export default function Page() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const prefix = searchParams.get("prefix") ?? "";
	const router = useRouter();

	const fileToUrl = useCallback(
		async (file: string) => {
			const results = await backend.storageState.downloadStorageItemsUser(
				id ?? "",
				[file],
			);
			if (results.length > 0 && results[0].url) {
				return results[0].url;
			}
			return "";
		},
		[id, backend.storageState],
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
			title={t('userStorage', 'User Storage')}
			storageScopeKey="user"
			operations={operations}
			updatePrefix={(nextPrefix) => {
				router.push(
					`/library/config/user-storage?id=${id}&prefix=${encodeURIComponent(nextPrefix)}`,
				);
			}}
			key={`${id}-${prefix}`}
		/>
	);
}
