"use client";
import { i18n as i18next } from "@flow-like/locales";
import { LayoutTemplate } from "lucide-react";
import { forwardRef } from "react";
import { Button } from "../../ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";

interface FlowTemplateButtonProps {
	onClick: () => void;
	className?: string;
}

export const FlowTemplateButton = forwardRef<
	HTMLButtonElement,
	FlowTemplateButtonProps
>(({ onClick, className }, ref) => {
	return (
		<TooltipProvider>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						ref={ref}
						variant="outline"
						size="sm"
						onClick={onClick}
						className={`h-8 gap-1.5 px-2.5 border-border/50 hover:border-primary/30 hover:bg-primary/5 ${className}`}
					>
						<LayoutTemplate className="h-3.5 w-3.5 text-primary" />
						<span className="text-xs font-medium hidden sm:inline">
							{i18next.t("templates", "Templates")}
						</span>
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					<p>{i18next.t("browseTemplates", "Browse templates")}</p>
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
});

FlowTemplateButton.displayName = "FlowTemplateButton";

export default FlowTemplateButton;
