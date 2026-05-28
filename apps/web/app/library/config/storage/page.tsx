"use client";
import { StorageSystem, useBackend } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";

export default function Page() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const prefix = searchParams.get("prefix") ?? "";
	const router = useRouter();

	const fileToUrl = useCallback(
		async (file: string) => {
			// In web mode, download the storage item and get the signed URL
			const filePath = file.split("/").slice(3).join("/");
			const results = await backend.storageState.downloadStorageItems(
				id ?? "",
				[filePath],
			);
			if (results.length > 0 && results[0].url) {
				return results[0].url;
			}
			return "";
		},
		[id, backend.storageState],
	);

	return (
		<StorageSystem
			appId={id ?? ""}
			prefix={decodeURIComponent(prefix)}
			fileToUrl={fileToUrl}
			updatePrefix={(prefix) => {
				router.push(
					`/library/config/storage?id=${id}&prefix=${encodeURIComponent(prefix)}`,
				);
			}}
			key={`${id}-${prefix}`}
		/>
	);
}
