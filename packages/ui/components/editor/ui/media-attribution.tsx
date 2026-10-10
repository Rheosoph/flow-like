import { type MediaMetadata, mediaAttribution } from "../media-metadata";

export function MediaAttribution({ element }: { element: MediaMetadata }) {
	const label = mediaAttribution(element);
	return label ? (
		<div
			className="mt-1 text-xs text-muted-foreground"
			data-media-credit={element.credit}
			data-media-license={element.license}
		>
			{label}
		</div>
	) : null;
}
