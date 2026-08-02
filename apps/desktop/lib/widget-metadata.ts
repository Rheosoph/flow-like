import type { IMetadata, ISystemTime, IWidget } from "@flow-like/flow-like-ui";

function isoToSystemTime(iso: string | undefined): ISystemTime {
	const parsed = iso ? Date.parse(iso) : Number.NaN;
	return {
		nanos_since_epoch: 0,
		secs_since_epoch: Math.floor(
			(Number.isFinite(parsed) ? parsed : Date.now()) / 1000,
		),
	};
}

/**
 * Widgets created outside the widget config page have no metadata sidecar, so
 * every listing that reads names from metadata fell back to the raw widget id.
 * The widget carries its own name — use it whenever metadata supplies none.
 */
export function withWidgetName(
	metadata: IMetadata | undefined,
	widget: IWidget | undefined,
): IMetadata | undefined {
	if (metadata?.name?.trim() || !widget) return metadata;
	const name = widget.name?.trim();
	if (!name) return metadata;
	return {
		...metadata,
		name,
		description: metadata?.description || widget.description || "",
		long_description: metadata?.long_description ?? "",
		tags: metadata?.tags ?? widget.tags ?? [],
		preview_media: metadata?.preview_media ?? [],
		created_at: metadata?.created_at ?? isoToSystemTime(widget.createdAt),
		updated_at: metadata?.updated_at ?? isoToSystemTime(widget.updatedAt),
	};
}
