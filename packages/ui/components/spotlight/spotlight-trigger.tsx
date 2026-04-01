"use client";

import { CommandIcon, Search, Sparkles, SparklesIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import { useSpotlightStore } from "../../state/spotlight-state";
import { Button } from "../ui/button";
import {
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	useSidebar,
} from "../ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

interface SpotlightTriggerProps {
	className?: string;
	variant?: "button" | "sidebar" | "minimal";
}

import { motion } from "framer-motion";

const MotionSidebarMenuButton = motion.create(SidebarMenuButton);

const iconVariants = {
	initial: { scale: 1, rotate: 0 },
	hover: {
		scale: 1.1,
		rotate: 5,
		transition: { type: "spring", stiffness: 400, damping: 10 },
	},
};

export function SpotlightTrigger({
	className,
	variant = "sidebar",
}: SpotlightTriggerProps) {
	const { open } = useSpotlightStore();
	const { open: sidebarOpen } = useSidebar();

	if (variant === "button") {
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="outline"
						size="sm"
						onClick={open}
						className={cn(
							"relative gap-2 text-muted-foreground hover:text-foreground",
							"transition-all duration-200",
							className,
						)}
					>
						<Search className="h-4 w-4" />
						<span className="hidden sm:inline-flex">Search...</span>
						<kbd className="pointer-events-none hidden h-5 select-none items-center gap-1 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium opacity-100 sm:flex">
							<CommandIcon className="h-3 w-3" />K
						</kbd>
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom">
					<p>Open Spotlight</p>
					<p className="text-xs text-muted-foreground">⌘K to open</p>
				</TooltipContent>
			</Tooltip>
		);
	}

	if (variant === "minimal") {
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						onClick={open}
						className={cn("h-9 w-9", className)}
					>
						<Sparkles className="h-6 w-6 text-primary" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="right">
					<p>Open Spotlight</p>
					<p className="text-xs text-muted-foreground">⌘K</p>
				</TooltipContent>
			</Tooltip>
		);
	}

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<MotionSidebarMenuButton
					onClick={open}
					tooltip="Quick Search (⌘K)"
					className={cn(
						"group relative transition-all duration-200 bg-primary/50 border-primary border-1",
						"hover:bg-accent/80",
						className,
					)}
					initial="initial"
					whileHover="hover"
				>
					<motion.div variants={iconVariants} className="flex items-center justify-center h-4 w-4 rounded-sm bg-linear-to-br">
						<SparklesIcon className="h-5 w-5 text-primary-foreground" />
					</motion.div>
					{sidebarOpen && (
						<>
							<span className="flex-1">Quick Search</span>
							<kbd className="pointer-events-none inline-flex h-5 select-none items-center gap-0.5 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground">
								<CommandIcon className="h-3 w-3" />K
							</kbd>
						</>
					)}
				</MotionSidebarMenuButton>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}
