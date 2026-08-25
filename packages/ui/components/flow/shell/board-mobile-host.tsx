"use client";

import type { ReactNode } from "react";
import { memo } from "react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "../../ui/sheet";

/**
 * The single drawer below `md`.
 *
 * Every surface used to ship twice — a docked panel and its own `Sheet`, behind
 * its own boolean. Here the shell's surface state decides what is open and this
 * host renders it, so a surface is written once and placed by the layout.
 */
export const BoardMobileHost = memo(function BoardMobileHost({
	open,
	title,
	onClose,
	children,
	full,
}: Readonly<{
	open: boolean;
	title: string;
	onClose: () => void;
	children: ReactNode;
	full?: boolean;
}>) {
	return (
		<Sheet
			open={open}
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
		>
			<SheetContent
				side="bottom"
				className={full ? "h-[100dvh] w-full p-0" : "h-[85dvh] w-full p-0"}
			>
				<SheetHeader className="px-4 pt-4">
					<SheetTitle>{title}</SheetTitle>
				</SheetHeader>
				<div className="min-h-0 flex-1 overflow-auto">{children}</div>
			</SheetContent>
		</Sheet>
	);
});
