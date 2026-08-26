"use client";

import { useTranslation } from "@flow-like/locales";
import { XIcon } from "lucide-react";
import type { ReactNode } from "react";
import { createContext, memo, useContext, useState } from "react";
import { cn } from "../../../lib/utils";

/**
 * The chrome every docked surface wears: a 28 px header with the view's name,
 * its own actions, and one close affordance. Sidebar, secondary sidebar and the
 * mobile drawer all render through this, so a surface looks the same wherever
 * the shell happens to put it.
 */
export const BoardPane = memo(function BoardPane({
	title,
	actions,
	onClose,
	children,
	className,
	bodyClassName,
}: Readonly<{
	title: string;
	actions?: ReactNode;
	onClose?: () => void;
	children: ReactNode;
	className?: string;
	bodyClassName?: string;
}>) {
	const { t } = useTranslation("flow");
	return (
		<section
			className={cn("flex h-full min-h-0 flex-col bg-background", className)}
		>
			<header className="flex h-7 shrink-0 items-center gap-1 border-b px-2">
				<h2 className="truncate text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
					{title}
				</h2>
				<span className="flex-1" />
				{actions}
				{onClose && (
					<button
						type="button"
						onClick={onClose}
						aria-label={t("close", "Close")}
						className="flex size-5 items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground"
					>
						<XIcon className="size-3.5" />
					</button>
				)}
			</header>
			<div className={cn("min-h-0 flex-1 overflow-auto", bodyClassName)}>
				{children}
			</div>
		</section>
	);
});

/**
 * The panel's tab strip lends its right-hand side to whichever view is showing.
 *
 * A short panel cannot afford a row of tabs *and* a row of filters, and the
 * controls belong to the view, not to the shell — so the view keeps its own
 * state and portals its toolbar up here. Only the mounted tab has a toolbar, so
 * switching tabs swaps the controls with no wiring in between.
 */
const BoardPanelToolbarContext = createContext<HTMLElement | null>(null);

export function usePanelToolbarSlot(): HTMLElement | null {
	return useContext(BoardPanelToolbarContext);
}

export interface IBoardPanelTabDescriptor {
	id: string;
	label: string;
	badge?: number;
	badgeTone?: "default" | "warning" | "danger";
}

/**
 * The bottom panel — Problems, Runs and Traces in one tab strip instead of a
 * side panel, a nested vertical panel and a sheet that never knew about each
 * other.
 */
export const BoardPanel = memo(function BoardPanel({
	tabs,
	active,
	onSelect,
	onClose,
	actions,
	children,
}: Readonly<{
	tabs: IBoardPanelTabDescriptor[];
	active: string;
	onSelect: (id: string) => void;
	onClose: () => void;
	actions?: ReactNode;
	children: ReactNode;
}>) {
	const { t } = useTranslation("flow");
	const [toolbar, setToolbar] = useState<HTMLElement | null>(null);
	return (
		<section className="flex h-full min-h-0 flex-col bg-background">
			<div className="flex h-8 shrink-0 items-stretch border-b">
				{tabs.map((tab) => (
					<button
						key={tab.id}
						type="button"
						onClick={() => onSelect(tab.id)}
						aria-pressed={tab.id === active}
						className={cn(
							"relative flex items-center gap-1.5 px-3 text-[11px] font-medium uppercase tracking-wider transition-colors",
							tab.id === active
								? "text-foreground"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						{tab.label}
						{typeof tab.badge === "number" && tab.badge > 0 && (
							<span
								className={cn(
									"tabular-nums",
									tab.badgeTone === "danger" && "text-destructive",
									tab.badgeTone === "warning" && "text-amber-500",
								)}
							>
								{tab.badge}
							</span>
						)}
						{tab.id === active && (
							<span className="absolute inset-x-2 bottom-0 h-0.5 rounded-t bg-primary" />
						)}
					</button>
				))}
				<span className="flex-1" />
				<div className="flex min-w-0 items-center gap-1 pr-1.5">
					<div
						ref={setToolbar}
						className="flex min-w-0 items-center gap-1 overflow-x-auto no-scrollbar"
					/>
					{actions}
					<button
						type="button"
						onClick={onClose}
						aria-label={t("close", "Close")}
						className="flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground"
					>
						<XIcon className="size-3.5" />
					</button>
				</div>
			</div>
			<div className="min-h-0 flex-1 overflow-hidden">
				<BoardPanelToolbarContext.Provider value={toolbar}>
					{children}
				</BoardPanelToolbarContext.Provider>
			</div>
		</section>
	);
});
