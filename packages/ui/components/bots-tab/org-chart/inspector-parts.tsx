"use client";

import type { ReactNode } from "react";
import { cn } from "../../../lib/utils";

export function InspectorSection({
	title,
	hint,
	action,
	children,
	className,
}: Readonly<{
	title: string;
	hint?: string;
	action?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	return (
		<section className={cn("flex flex-col gap-2", className)}>
			<div className="flex items-center justify-between gap-2">
				<h3 className="text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
					{title}
				</h3>
				{action}
			</div>
			{hint && <p className="-mt-1 text-xs text-muted-foreground">{hint}</p>}
			{children}
		</section>
	);
}

export function Chip({
	children,
	onClick,
	selected,
	className,
	title,
}: Readonly<{
	children: ReactNode;
	onClick?: () => void;
	selected?: boolean;
	className?: string;
	title?: string;
}>) {
	return (
		<button
			type="button"
			title={title}
			onClick={onClick}
			aria-pressed={selected}
			className={cn(
				"flex max-w-full items-center gap-1.5 rounded-full border bg-card py-0.5 pr-2.5 pl-0.5 text-xs outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
				selected && "border-primary bg-primary/10",
				className,
			)}
		>
			{children}
		</button>
	);
}
