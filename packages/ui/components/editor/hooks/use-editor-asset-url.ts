"use client";

import { useContext, useEffect, useState } from "react";
import { signedUrlExpiry } from "../../../lib/stable-asset-url";
import { useBackend } from "../../../state/backend-state";
import type { IStorageState } from "../../../state/backend-state/storage-state";
import {
	type EditorAssetResolver,
	EditorAssetResolverContext,
} from "../asset-resolver-context";
import {
	isStorageUrl,
	parseEditorStorageReference,
	useEditorUpload,
} from "../upload-context";

type StorageReader = Pick<
	IStorageState,
	"downloadStorageItems" | "downloadStorageItemsUser"
>;

/** Resolve through the same storage area used by the uploader. */
export async function resolveEditorAssetUrl(
	url: string,
	storage: StorageReader,
	appId?: string,
	scope: "app" | "user" = "app",
): Promise<string> {
	if (!isStorageUrl(url)) return url;
	const reference = parseEditorStorageReference(url, appId, scope);
	if (!reference?.appId || !reference.path)
		throw new Error("The media reference has no storage owner.");
	const download =
		reference.scope === "user"
			? storage.downloadStorageItemsUser
			: storage.downloadStorageItems;
	const results = await download.call(storage, reference.appId, [
		reference.path,
	]);
	const resolved = results[0]?.url;
	if (!resolved) throw new Error(`Media is unavailable: ${reference.path}`);
	return resolved;
}

/** Private signed URLs remain local to the mounted media component. */
export function useEditorAssetUrl(url: string | undefined): string | undefined {
	const backend = useBackend();
	const { appId, scope } = useEditorUpload();
	const hostResolver = useContext(EditorAssetResolverContext);
	const resolver = url && hostResolver?.accepts(url) ? hostResolver : undefined;
	const needsResolution = Boolean(resolver || isStorageUrl(url));
	const [asset, setAsset] = useState<{
		source: string;
		url: string;
		resolver?: EditorAssetResolver;
		storage: IStorageState;
	}>();
	const source = `${appId ?? ""}:${scope}:${url ?? ""}`;

	useEffect(() => {
		if (!url || !needsResolution) return;
		let cancelled = false;
		let timer: ReturnType<typeof setTimeout>;
		const refresh = async () => {
			try {
				const resolved = await (resolver
					? resolver.resolve(url)
					: resolveEditorAssetUrl(url, backend.storageState, appId, scope));
				if (cancelled) return;
				setAsset({
					source,
					url: resolved,
					resolver,
					storage: backend.storageState,
				});
				const expiry = signedUrlExpiry(resolved);
				const delay = Math.max(
					30_000,
					Math.min(
						25 * 60_000,
						expiry ? expiry - Date.now() - 60_000 : Number.POSITIVE_INFINITY,
					),
				);
				timer = setTimeout(refresh, delay);
			} catch {
				if (cancelled) return;
				setAsset(undefined);
				timer = setTimeout(refresh, 30_000);
			}
		};
		void refresh();
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [
		url,
		appId,
		scope,
		source,
		backend.storageState,
		resolver,
		needsResolution,
	]);

	if (!needsResolution) return url;
	return asset?.source === source &&
		asset.resolver === resolver &&
		asset.storage === backend.storageState
		? asset.url
		: undefined;
}
