"use client";

import { ChevronRightIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { cn } from "../../../lib/utils";

/**
 * Where the canvas is, inside the open file.
 *
 * A layer used to announce itself with a label painted over the bottom-left of
 * the graph, which said where you were but offered no way out except Layer Up.
 * Here every ancestor is a target, so leaving a nested layer is one click at any
 * depth.
 *
 * It renders nothing at the file root: the tab strip already names the file, and
 * a row of chrome that says only what the row above says is not worth the pixels.
 */
export const BoardBreadcrumb = memo(function BoardBreadcrumb({
	fileLabel,
	layerPath,
	layerNames,
	onJumpToLayer,
}: Readonly<{
	fileLabel: string;
	/** Layer ids joined by `/`, deepest last. Undefined at the file root. */
	layerPath?: string;
	layerNames: Map<string, string>;
	onJumpToLayer: (path: string) => void;
}>) {
	const segments = useMemo(
		() => (layerPath ? layerPath.split("/").filter(Boolean) : []),
		[layerPath],
	);

	if (segments.length === 0) return null;

	return (
		<nav
			aria-label={fileLabel}
			className="flex h-5 shrink-0 items-center gap-0.5 overflow-x-auto border-b bg-muted/10 px-2 no-scrollbar"
		>
			<button
				type="button"
				onClick={() => onJumpToLayer("root")}
				className="shrink-0 rounded-sm px-1 font-mono text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
			>
				{fileLabel}
			</button>
			{segments.map((segment, index) => {
				const last = index === segments.length - 1;
				const path = segments.slice(0, index + 1).join("/");
				return (
					<span key={segment} className="flex shrink-0 items-center gap-0.5">
						<ChevronRightIcon className="size-3 text-muted-foreground/50" />
						<button
							type="button"
							disabled={last}
							aria-current={last ? "page" : undefined}
							onClick={() => onJumpToLayer(path)}
							className={cn(
								"rounded-sm px-1 text-[11px]",
								last
									? "text-foreground"
									: "text-muted-foreground hover:bg-accent hover:text-foreground",
							)}
						>
							{layerNames.get(segment) ?? segment}
						</button>
					</span>
				);
			})}
		</nav>
	);
});
