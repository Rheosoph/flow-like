import type { LessonAssetView } from "./types";

/**
 * Custom field on plate elements that marks them as backed by a course asset.
 * The `url` on the saved node is a stale signed URL — at render time we look
 * the asset up by name and overwrite `url` with a fresh signed URL.
 */
export const ASSET_NAME_FIELD = "assetName";

interface AssetPlateNode {
	readonly type: string;
	readonly url: string;
	readonly assetName: string;
	readonly children: ReadonlyArray<{ readonly text: string }>;
}

interface AssetLinkNode {
	readonly type: "a";
	readonly url: string;
	readonly assetName: string;
	readonly children: ReadonlyArray<{ readonly text: string }>;
}

/**
 * Build the plate element to insert when the lesson author picks `@<asset>`
 * from the mention combobox. Image/video/audio render inline as their native
 * media element; documents become a link with the asset name as label.
 */
export function buildAssetPlateNode(
	asset: LessonAssetView,
): AssetPlateNode | AssetLinkNode {
	switch (asset.kind) {
		case "IMAGE":
			return {
				type: "img",
				url: asset.signed_url,
				assetName: asset.name,
				children: [{ text: "" }],
			};
		case "VIDEO":
			return {
				type: "video",
				url: asset.signed_url,
				assetName: asset.name,
				children: [{ text: "" }],
			};
		case "AUDIO":
			return {
				type: "audio",
				url: asset.signed_url,
				assetName: asset.name,
				children: [{ text: "" }],
			};
		default:
			return {
				type: "a",
				url: asset.signed_url,
				assetName: asset.name,
				children: [{ text: asset.name }],
			};
	}
}
