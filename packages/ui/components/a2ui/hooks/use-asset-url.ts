"use client";

import {
	type AssetSourceOptions,
	type AssetSourceState,
	useAssetSource,
} from "../../../hooks/use-asset-source";
import { useActionContext } from "../ActionHandler";

/**
 * Resolves an a2ui component's `src` against the surface's own app.
 *
 * Components hold durable storage paths; the signed URL that actually loads is
 * minted here and renewed before it lapses. See
 * `packages/ui/lib/asset-url-cache.ts` for why the path, and not the URL, is
 * what a surface stores.
 */
export function useAssetUrl(
	assetPath: string | undefined,
	options?: AssetSourceOptions,
): {
	url: string | undefined;
	isLoading: boolean;
	refresh: AssetSourceState["refresh"];
} {
	const { appId } = useActionContext();
	const { src, isLoading, refresh } = useAssetSource(appId, assetPath, options);
	return { url: src, isLoading, refresh };
}
