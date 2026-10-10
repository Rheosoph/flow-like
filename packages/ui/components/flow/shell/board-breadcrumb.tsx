"use client";

import { ChevronRightIcon } from "lucide-react";
import { memo, useMemo } from "react";
import {
	type LayerVisit,
	resolveLayerChain,
} from "../../../hooks/use-layer-navigation";
import {
	MAIN_FILE_LABEL,
	activeModuleId,
	modulePathLabel,
} from "../../../lib/flow-modules";
import { type ILayer, ILayerType } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";

interface BreadcrumbEntry {
	label: string;
	path: string;
	visitIndex?: number;
}

/** The route into the open function, with ownership ancestors before the first visit. */
export const BoardBreadcrumb = memo(function BoardBreadcrumb({
	layers,
	layerPath,
	navigationTrail,
	onReturnToVisit,
	onJumpToLayer,
}: Readonly<{
	layers: Record<string, ILayer>;
	/** Full ownership path from the board root, deepest last. */
	layerPath?: string;
	navigationTrail?: readonly LayerVisit[];
	onReturnToVisit?: (index: number) => void;
	onJumpToLayer: (path: string) => void;
}>) {
	const entries = useMemo(() => {
		const trail =
			navigationTrail?.length && navigationTrail.at(-1)?.to === layerPath
				? navigationTrail
				: undefined;
		const origin = trail ? trail[0].from : layerPath;
		const originChain = resolveLayerChain(layers, origin?.split("/").pop());
		const originModule = activeModuleId(origin, originChain.at(-1), layers);
		const fileChain = resolveLayerChain(layers, originModule);
		const fileLabel = originModule
			? modulePathLabel(layers, originModule)
			: MAIN_FILE_LABEL;
		const result: BreadcrumbEntry[] = [
			{
				label: fileLabel,
				path: fileChain.join("/") || "root",
				visitIndex:
					trail && originChain.length === fileChain.length ? 0 : undefined,
			},
			...originChain.slice(fileChain.length).map((id, index) => ({
				label: layers[id]?.name ?? id,
				path: originChain.slice(0, fileChain.length + index + 1).join("/"),
				visitIndex:
					trail && fileChain.length + index === originChain.length - 1
						? 0
						: undefined,
			})),
		];

		let previousModule = originModule;
		for (const [index, visit] of (trail ?? []).entries()) {
			const chain = resolveLayerChain(layers, visit.to.split("/").pop());
			const layerId = chain.at(-1);
			const layer = layerId ? layers[layerId] : undefined;
			const moduleId = activeModuleId(visit.to, layerId, layers);
			const moduleLabel = moduleId
				? modulePathLabel(layers, moduleId)
				: MAIN_FILE_LABEL;
			const name = layer?.name ?? layerId ?? MAIN_FILE_LABEL;
			result.push({
				label:
					layer?.type === ILayerType.Module
						? moduleLabel
						: moduleId !== previousModule
							? `${moduleLabel}: ${name}`
							: name,
				path: chain.join("/") || "root",
				visitIndex: index + 1,
			});
			previousModule = moduleId;
		}
		return result;
	}, [layers, layerPath, navigationTrail]);

	if (entries.length < 2) return null;

	return (
		<nav
			aria-label={entries[0].label}
			className="flex h-5 shrink-0 items-center gap-0.5 overflow-x-auto border-b bg-muted/10 px-2 no-scrollbar"
		>
			{entries.map(({ label, path, visitIndex }, index) => {
				const last = index === entries.length - 1;
				return (
					<span
						key={`${index}:${path}`}
						className="flex shrink-0 items-center gap-0.5"
					>
						{index > 0 && (
							<ChevronRightIcon className="size-3 text-muted-foreground/50" />
						)}
						<button
							type="button"
							disabled={last}
							aria-current={last ? "page" : undefined}
							onClick={() => {
								if (visitIndex !== undefined && onReturnToVisit) {
									onReturnToVisit(visitIndex);
								} else {
									onJumpToLayer(path);
								}
							}}
							className={cn(
								"rounded-sm px-1 text-[11px]",
								index === 0 && "font-mono",
								last
									? "text-foreground"
									: "text-muted-foreground hover:bg-accent hover:text-foreground",
							)}
						>
							{label}
						</button>
					</span>
				);
			})}
		</nav>
	);
});
