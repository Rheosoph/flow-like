"use client";

import ArrowUpRightIcon from "lucide-react/dist/esm/icons/arrow-up-right.js";
import LayersIcon from "lucide-react/dist/esm/icons/layers.js";
import { useRouter } from "next/navigation";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IMetadata } from "../../../lib";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../../ui/hover-card";
import { Skeleton } from "../../ui/skeleton";

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

const CHIP_VARIANTS = {
	pill: "rounded-full bg-background/70 py-1 pl-1.5 pr-2 text-xs text-foreground/80",
	inline:
		"rounded-md bg-background/50 px-2 py-1 text-[11px] text-muted-foreground",
};

/** An app by name, never by id: hover shows what the app is, click opens it. */
export function AppReferenceChip({
	appId,
	fallbackName,
	variant = "pill",
}: Readonly<{
	appId: string;
	fallbackName?: string;
	variant?: keyof typeof CHIP_VARIANTS;
}>) {
	const backend = useBackend();
	const router = useRouter();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		true,
	);
	const name = meta.data?.name || fallbackName || "App";
	const icon = meta.data?.icon ?? meta.data?.thumbnail;
	const open = () => router.push(`/use?id=${encodeURIComponent(appId)}`);

	return (
		<HoverCard openDelay={250} closeDelay={100}>
			<HoverCardTrigger asChild>
				<button
					type="button"
					onClick={open}
					data-app-reference={appId}
					className={cn(
						"group flex w-fit min-w-0 max-w-full items-center gap-1.5 border border-border/60 transition-all hover:border-primary/40 hover:bg-primary/10 hover:text-primary hover:ring-1 hover:ring-primary/30 motion-safe:active:scale-98 outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0",
						CHIP_VARIANTS[variant],
					)}
				>
					<AppMark icon={icon} />
					<span
						className={cn(
							"min-w-0 truncate font-medium",
							variant === "pill" ? "max-w-36" : "text-foreground/80",
						)}
					>
						{name}
					</span>
					<ArrowUpRightIcon
						className="size-3 shrink-0 opacity-50 transition-opacity group-hover:opacity-100"
						aria-hidden="true"
					/>
				</button>
			</HoverCardTrigger>
			<HoverCardContent align="start" className="w-72 overflow-hidden p-0">
				<AppPreview
					meta={meta.data}
					name={name}
					icon={icon}
					isLoading={meta.isLoading}
					onOpen={open}
				/>
			</HoverCardContent>
		</HoverCard>
	);
}

function AppMark({
	icon,
	large = false,
}: Readonly<{ icon?: string | null; large?: boolean }>) {
	const size = large ? "size-9 rounded-lg" : "size-4 rounded-sm";
	return icon ? (
		<img src={icon} alt="" className={cn(size, "shrink-0 object-cover")} />
	) : (
		<span
			className={cn(
				size,
				"flex shrink-0 items-center justify-center bg-primary/15 text-primary",
			)}
		>
			<LayersIcon className={large ? "size-4" : "size-2.5"} />
		</span>
	);
}

function AppPreview({
	meta,
	name,
	icon,
	isLoading,
	onOpen,
}: Readonly<{
	meta?: IMetadata;
	name: string;
	icon?: string | null;
	isLoading: boolean;
	onOpen: () => void;
}>) {
	const description = meta?.description?.trim() || meta?.use_case?.trim();
	const tags = meta?.tags?.slice(0, 3) ?? [];
	return (
		<div className="flex flex-col">
			{meta?.thumbnail && meta.thumbnail !== icon && (
				<img
					src={meta.thumbnail}
					alt=""
					className="aspect-[5/2] w-full border-b border-border/60 object-cover"
				/>
			)}
			<div className="flex flex-col gap-2.5 p-3">
				<div className="flex min-w-0 items-center gap-2.5">
					<AppMark icon={icon} large />
					<div className="min-w-0">
						<p className="truncate text-sm font-semibold">{name}</p>
						{tags.length > 0 && (
							<p className="truncate text-[11px] text-muted-foreground">
								{tags.join(" · ")}
							</p>
						)}
					</div>
				</div>
				{isLoading ? (
					<div className="space-y-1.5">
						<Skeleton className="h-3 w-full" />
						<Skeleton className="h-3 w-2/3" />
					</div>
				) : description ? (
					<p className="line-clamp-3 text-xs leading-relaxed text-muted-foreground">
						{description}
					</p>
				) : !meta ? (
					<p className="text-xs text-muted-foreground">
						This app's details aren't available to you.
					</p>
				) : null}
				<Button
					size="sm"
					variant="secondary"
					className="h-7 w-full gap-1.5 text-xs"
					onClick={onOpen}
				>
					Open app
					<ArrowUpRightIcon className="size-3" aria-hidden="true" />
				</Button>
			</div>
		</div>
	);
}
