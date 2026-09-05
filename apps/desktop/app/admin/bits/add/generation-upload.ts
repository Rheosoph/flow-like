import type { GenerationAssetDraft, IBit } from "@flow-like/flow-like-ui";

/** Retain completed registrations so a failed upload can resume at the next file. */
export async function registerGenerationAssets({
	assets,
	completed,
	uploadBit,
	onProgress,
	onRegistered,
}: {
	assets: GenerationAssetDraft[];
	completed: Map<string, string>;
	uploadBit: (bit: IBit) => Promise<IBit>;
	onProgress: (asset: GenerationAssetDraft, index: number) => void;
	onRegistered: (asset: GenerationAssetDraft) => void;
}): Promise<GenerationAssetDraft[]> {
	const registered: GenerationAssetDraft[] = [];
	for (const [index, asset] of assets.entries()) {
		if (completed.get(asset.key) === JSON.stringify(asset.bit)) {
			registered.push(asset);
			continue;
		}
		onProgress(asset, index);
		const bit = await uploadBit(asset.bit);
		const nextAsset = { ...asset, bit };
		completed.set(asset.key, JSON.stringify(bit));
		registered.push(nextAsset);
		onRegistered(nextAsset);
	}
	return registered;
}
