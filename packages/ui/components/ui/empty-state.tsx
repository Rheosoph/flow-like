import { useTranslation } from "@flow-like/locales";
import type { LucideIcon } from "lucide-react";
import * as React from "react";
import { Button } from "../../components/ui/button";
import { cn } from "../../lib/utils";
import { FloatingOrbs } from "./flow-background";

interface EmptyStateProps {
	title: string;
	description: string;
	icons?: LucideIcon[];
	action?:
		| {
				label: string;
				onClick: () => void;
		  }
		| {
				label: string;
				onClick: () => void;
		  }[];
	className?: string;
}

export function EmptyState({
	title,
	description,
	icons = [],
	action,
	className,
}: Readonly<EmptyStateProps>) {
	const { t } = useTranslation("common");
	return (
		<div
			className={cn(
				"bg-background/50 backdrop-blur-sm border-border/50 hover:border-primary/20 text-center relative overflow-hidden",
				"border-2 border-dashed rounded-xl p-14 w-full max-w-[620px]",
				"group hover:bg-muted/30 transition-all duration-500 hover:duration-200",
				className,
			)}
		>
			{/* Floating orbs background */}
			<FloatingOrbs
				count={3}
				className="opacity-30 group-hover:opacity-50 transition-opacity duration-500"
			/>

			{/* Gradient accent on hover */}
			<div className="absolute inset-0 bg-gradient-to-br from-primary/[0.03] via-transparent to-purple-500/[0.03] opacity-0 group-hover:opacity-100 transition-opacity duration-500 pointer-events-none" />

			<div className="relative z-10 flex justify-center isolate">
				{icons.length === 3 ? (
					<>
						<div className="bg-background/80 backdrop-blur-sm size-12 grid place-items-center rounded-xl relative left-2.5 top-1.5 -rotate-6 shadow-lg ring-1 ring-border/50 group-hover:ring-primary/20 group-hover:-translate-x-5 group-hover:-rotate-12 group-hover:-translate-y-0.5 transition-all duration-500 group-hover:duration-200">
							{React.createElement(icons[0], {
								className: `w-6 h-6 text-muted-foreground`,
							})}
						</div>
						<div className="bg-background/80 backdrop-blur-sm size-12 grid place-items-center rounded-xl relative z-10 shadow-lg ring-1 ring-border/50 group-hover:ring-primary/20 group-hover:-translate-y-0.5 transition-all duration-500 group-hover:duration-200">
							{React.createElement(icons[1], {
								className: `w-6 h-6 text-muted-foreground`,
							})}
						</div>
						<div className="bg-background/80 backdrop-blur-sm size-12 grid place-items-center rounded-xl relative right-2.5 top-1.5 rotate-6 shadow-lg ring-1 ring-border/50 group-hover:ring-primary/20 group-hover:translate-x-5 group-hover:rotate-12 group-hover:-translate-y-0.5 transition-all duration-500 group-hover:duration-200">
							{React.createElement(icons[2], {
								className: `w-6 h-6 text-muted-foreground`,
							})}
						</div>
					</>
				) : (
					<div className="bg-background/80 backdrop-blur-sm size-12 grid place-items-center rounded-xl shadow-lg ring-1 ring-border/50 group-hover:ring-primary/20 group-hover:-translate-y-0.5 transition-all duration-500 group-hover:duration-200">
						{icons[0] &&
							React.createElement(icons[0], {
								className: `w-6 h-6 text-muted-foreground`,
							})}
					</div>
				)}
			</div>
			<h2 className="relative z-10 text-foreground font-medium mt-6">
				{title}
			</h2>
			<p className="relative z-10 text-sm text-muted-foreground mt-1 whitespace-pre-line">
				{description}
			</p>
			{action && !Array.isArray(action) && (
				<Button
					onClick={action.onClick}
					variant="outline"
					className={cn("relative z-10 mt-4", "shadow-sm active:shadow-none")}
				>
					{action.label}
				</Button>
			)}

			{action && Array.isArray(action) && (
				<div className="relative z-10 flex flex-row justify-center gap-2 mt-4">
					{action.map((a) => (
						<Button
							key={a.label}
							onClick={a.onClick}
							variant="outline"
							className={cn("shadow-sm active:shadow-none")}
						>
							{a.label}
						</Button>
					))}
				</div>
			)}
		</div>
	);
}
