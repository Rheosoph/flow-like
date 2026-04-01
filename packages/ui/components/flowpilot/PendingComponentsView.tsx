"use client";

import { AnimatePresence, motion } from "framer-motion";
import {
	AlertTriangleIcon,
	ChevronDown,
	EyeIcon,
	LayoutGridIcon,
	ListIcon,
	PlayIcon,
	XIcon,
} from "lucide-react";
import { memo, useMemo, useState } from "react";

import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import type { CanvasSettings, Surface, SurfaceComponent } from "../a2ui/types";
import { Button } from "../ui/button";
import { getComponentCounts } from "./utils";

/** Build a renderable Surface from a flat component array */
function buildPreviewSurface(
	components: SurfaceComponent[],
	canvasSettings?: CanvasSettings,
): Surface {
	const componentsRecord: Record<string, SurfaceComponent> = {};
	const referencedChildIds = new Set<string>();

	for (const comp of components) {
		componentsRecord[comp.id] = comp;
		const children = (comp.component as unknown as Record<string, unknown>)
			?.children as { explicitList?: string[] } | undefined;
		if (children?.explicitList) {
			for (const childId of children.explicitList) {
				referencedChildIds.add(childId);
			}
		}
	}

	// If there's a "root" component, use it; otherwise pick the first unreferenced component
	const rootComponentId =
		componentsRecord["root"] != null
			? "root"
			: components.find((c) => !referencedChildIds.has(c.id))?.id ??
				components[0]?.id ??
				"root";

	return {
		id: "preview",
		rootComponentId,
		components: componentsRecord,
		canvasSettings,
	};
}

interface PendingComponentsViewProps {
	components: SurfaceComponent[];
	canvasSettings?: CanvasSettings;
	warnings?: string[];
	onApply: () => void;
	onDismiss: () => void;
}

export const PendingComponentsView = memo(function PendingComponentsView({
	components,
	canvasSettings,
	warnings = [],
	onApply,
	onDismiss,
}: PendingComponentsViewProps) {
	const [isOpen, setIsOpen] = useState(true);
	const [showPreview, setShowPreview] = useState(true);
	const [showWarnings, setShowWarnings] = useState(false);

	const componentCounts = useMemo(
		() => getComponentCounts(components),
		[components],
	);

	const previewSurface = useMemo(
		() => buildPreviewSurface(components, canvasSettings),
		[components, canvasSettings],
	);

	if (components.length === 0) return null;

	return (
		<motion.div
			initial={{ opacity: 0, y: 10 }}
			animate={{ opacity: 1, y: 0 }}
			className="border-t border-border/30 bg-primary/5"
		>
			<div className="px-3 py-2.5">
				{/* Header */}
				<div className="flex items-center justify-between gap-2 mb-2">
					<div className="flex items-center gap-2">
						<div className="p-1 bg-primary/20 rounded-md">
							<LayoutGridIcon className="h-3 w-3 text-primary" />
						</div>
						<div>
							<div className="text-[10px] font-semibold">Ready to Apply</div>
							<div className="text-[9px] text-muted-foreground">
								{components.length} component
								{components.length !== 1 ? "s" : ""}
							</div>
						</div>
					</div>
					<div className="flex items-center gap-1">
						<Button
							size="sm"
							variant="ghost"
							className="h-6 w-6 p-0"
							onClick={() => setShowPreview((v) => !v)}
							title={showPreview ? "Show list" : "Show preview"}
						>
							{showPreview ? (
								<ListIcon className="h-3 w-3" />
							) : (
								<EyeIcon className="h-3 w-3" />
							)}
						</Button>
						<Button
							size="sm"
							variant="ghost"
							className="h-6 w-6 p-0"
							onClick={() => setIsOpen(!isOpen)}
						>
							<ChevronDown
								className={`h-3 w-3 transition-transform ${isOpen ? "rotate-180" : ""}`}
							/>
						</Button>
						<Button
							size="sm"
							className="h-6 px-2 text-[10px] gap-1"
							onClick={onApply}
						>
							<PlayIcon className="h-3 w-3" />
							Apply
						</Button>
						<Button
							size="sm"
							variant="ghost"
							className="h-6 w-6 p-0 text-muted-foreground hover:text-destructive"
							onClick={onDismiss}
						>
							<XIcon className="h-3 w-3" />
						</Button>
					</div>
				</div>

				{/* Component badges */}
				<div className="flex flex-wrap gap-1">
					{Object.entries(componentCounts).map(([type, count]) => (
						<span
							key={type}
							className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded-full text-[9px] font-medium bg-primary/10 text-primary border border-primary/20"
						>
							{count} {type}
						</span>
					))}
				</div>

				{/* Validation warnings */}
				{warnings.length > 0 && (
					<div className="mt-1.5">
						<button
							type="button"
							className="flex items-center gap-1 text-[9px] text-amber-500 hover:text-amber-400 transition-colors"
							onClick={() => setShowWarnings((v) => !v)}
						>
							<AlertTriangleIcon className="h-3 w-3" />
							{warnings.length} auto-fix{warnings.length !== 1 ? "es" : ""} applied
							<ChevronDown
								className={`h-2.5 w-2.5 transition-transform ${showWarnings ? "rotate-180" : ""}`}
							/>
						</button>
						{showWarnings && (
							<div className="mt-1 max-h-20 overflow-y-auto space-y-0.5 rounded-md bg-amber-500/5 border border-amber-500/20 p-1.5">
								{warnings.map((w) => (
									<div
										key={w}
										className="text-[8px] text-amber-500/80 leading-tight"
									>
										{w}
									</div>
								))}
							</div>
						)}
					</div>
				)}

				{/* Expanded content */}
				<AnimatePresence>
					{isOpen && (
						<motion.div
							initial={{ height: 0, opacity: 0 }}
							animate={{ height: "auto", opacity: 1 }}
							exit={{ height: 0, opacity: 0 }}
							transition={{ duration: 0.2 }}
							className="overflow-hidden"
						>
							{showPreview ? (
								<div className="pt-2 max-h-64 overflow-y-auto rounded-md border border-border/30 bg-background">
									<A2UIRenderer
										surface={previewSurface}
										isPreviewMode={true}
										className="w-full min-h-20 pointer-events-none scale-[0.85] origin-top-left"
									/>
								</div>
							) : (
								<div className="pt-2 space-y-1 max-h-24 overflow-y-auto">
									{components.map((comp, i) => (
										<div
											key={comp.id || i}
											className="flex items-center gap-2 p-1.5 rounded-md bg-background/50 text-[9px]"
										>
											<span className="font-medium">
												{comp.component?.type ?? "Unknown"}
											</span>
											<span className="text-muted-foreground truncate">
												{comp.id}
											</span>
										</div>
									))}
								</div>
							)}
						</motion.div>
					)}
				</AnimatePresence>
			</div>
		</motion.div>
	);
});
