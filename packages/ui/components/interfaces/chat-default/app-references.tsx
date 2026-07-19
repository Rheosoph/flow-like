"use client";

import { ArrowUpRightIcon, LayersIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";

/**
 * Clickable chips for the apps a message acted on/referenced (message.app_refs).
 * Clicking a chip opens the app's use surface.
 */
export function AppReferences({ appIds }: { appIds: string[] }) {
	if (appIds.length === 0) return null;
	return (
		<div className="flex flex-wrap items-center gap-1.5 mt-2">
			{appIds.map((appId) => (
				<AppReferenceChip key={appId} appId={appId} />
			))}
		</div>
	);
}

function AppReferenceChip({ appId }: { appId: string }) {
	const backend = useBackend();
	const router = useRouter();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		true,
	);

	const name = meta.data?.name || appId;
	const icon = meta.data?.icon ?? meta.data?.thumbnail;

	return (
		<button
			type="button"
			onClick={() => router.push(`/use?id=${appId}`)}
			title={meta.data?.description || name}
			className="group flex items-center gap-1.5 rounded-full border border-border/60 bg-background/70 py-1 pl-1.5 pr-2 text-xs text-foreground/80 transition-all hover:border-primary/40 hover:bg-primary/10 hover:text-primary hover:ring-1 hover:ring-primary/30 motion-safe:active:scale-98 outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
		>
			{icon ? (
				<img
					src={icon}
					alt=""
					className="size-4 rounded-sm object-cover shrink-0"
				/>
			) : (
				<span className="flex items-center justify-center size-4 rounded-sm bg-primary/15 text-primary shrink-0">
					<LayersIcon className="size-2.5" />
				</span>
			)}
			<span className="max-w-36 truncate font-medium">{name}</span>
			<ArrowUpRightIcon className="size-3 opacity-50 transition-opacity group-hover:opacity-100" />
		</button>
	);
}
