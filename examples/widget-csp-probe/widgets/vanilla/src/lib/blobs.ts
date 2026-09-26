import { wavBytes } from "./media";

/** Object URLs a sibling document keeps alive while it stays mounted */
export interface SiblingBlobUrls {
	/** `text/plain`, for the fetch check */
	text: string;
	/** `image/svg+xml`, for the img check */
	image: string;
	/** `audio/wav`, for the video check */
	media: string;
}

const MARKER_SVG =
	'<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#b91c1c"/></svg>';

export function createSiblingBlobs(): SiblingBlobUrls {
	const url = (content: BlobPart, type: string) =>
		URL.createObjectURL(new Blob([content], { type }));
	return {
		text: url("csp-probe-sibling blob contents", "text/plain"),
		image: url(MARKER_SVG, "image/svg+xml"),
		media: url(wavBytes(1), "audio/wav"),
	};
}
