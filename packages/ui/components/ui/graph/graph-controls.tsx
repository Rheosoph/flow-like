"use client";

import { useTranslation } from "@flow-like/locales";
import { Maximize, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { Button } from "../button";

export interface GraphControlsProps {
	onZoomIn?: () => void;
	onZoomOut?: () => void;
	onFitView?: () => void;
	onResetLayout?: () => void;
	compact?: boolean;
}

export function GraphControls({
	onZoomIn,
	onZoomOut,
	onFitView,
	onResetLayout,
	compact,
}: GraphControlsProps) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-col gap-1 bg-background/80 backdrop-blur-sm rounded-lg border p-1 shadow-sm">
			<Button
				variant="ghost"
				size="icon"
				className="h-8 w-8"
				onClick={onZoomIn}
				title={t("zoomIn", "Zoom in")}
			>
				<ZoomIn className="h-4 w-4" />
			</Button>
			<Button
				variant="ghost"
				size="icon"
				className="h-8 w-8"
				onClick={onZoomOut}
				title={t("zoomOut", "Zoom out")}
			>
				<ZoomOut className="h-4 w-4" />
			</Button>
			<Button
				variant="ghost"
				size="icon"
				className="h-8 w-8"
				onClick={onFitView}
				title={t("fitToView", "Fit to view")}
			>
				<Maximize className="h-4 w-4" />
			</Button>
			{onResetLayout && (
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8"
					onClick={onResetLayout}
					title={t("resetLayout", "Reset layout")}
				>
					<RotateCcw className="h-4 w-4" />
				</Button>
			)}
		</div>
	);
}
