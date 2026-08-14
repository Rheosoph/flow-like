"use client";

import { useTranslation } from "@flow-like/locales";
import * as SheetPrimitive from "@radix-ui/react-dialog";
import { XIcon } from "lucide-react";
import type * as React from "react";

import { cn } from "../../lib/utils";

function Sheet({ ...props }: React.ComponentProps<typeof SheetPrimitive.Root>) {
	return <SheetPrimitive.Root data-slot="sheet" {...props} />;
}

function SheetTrigger({
	...props
}: React.ComponentProps<typeof SheetPrimitive.Trigger>) {
	return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetClose({
	...props
}: React.ComponentProps<typeof SheetPrimitive.Close>) {
	return <SheetPrimitive.Close data-slot="sheet-close" {...props} />;
}

function SheetPortal({
	...props
}: React.ComponentProps<typeof SheetPrimitive.Portal>) {
	return <SheetPrimitive.Portal data-slot="sheet-portal" {...props} />;
}

function SheetOverlay({
	className,
	...props
}: React.ComponentProps<typeof SheetPrimitive.Overlay>) {
	return (
		<SheetPrimitive.Overlay
			data-slot="sheet-overlay"
			className={cn(
				"data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 fixed inset-0 z-50 bg-black/60 backdrop-blur-sm",
				className,
			)}
			{...props}
		/>
	);
}

function SheetContent({
	className,
	children,
	side = "right",
	style,
	overlayClassName,
	...props
}: React.ComponentProps<typeof SheetPrimitive.Content> & {
	side?: "top" | "right" | "bottom" | "left";
	/** Override the backdrop overlay (e.g. raise its z-index above a high-z host panel). */
	overlayClassName?: string;
}) {
	const { t } = useTranslation("common");
	const insetTop =
		side === "bottom"
			? "0px"
			: `var(--fl-safe-top, env(safe-area-inset-top, 0px))`;
	const insetBottom =
		side === "top"
			? "0px"
			: `var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px))`;

	const safeAreaStyle: React.CSSProperties = {
		paddingTop: insetTop,
		paddingBottom: insetBottom,
		...style,
	};

	return (
		<SheetPortal>
			<SheetOverlay className={overlayClassName} />
			<SheetPrimitive.Content
				data-slot="sheet-content"
				className={cn(
					"bg-background/95 backdrop-blur-md data-[state=open]:animate-in data-[state=closed]:animate-out fixed z-50 flex flex-col gap-4 shadow-2xl transition ease-in-out data-[state=closed]:duration-300 data-[state=open]:duration-500",
					side === "right" &&
						"data-[state=closed]:slide-out-to-right data-[state=open]:slide-in-from-right inset-y-0 right-0 h-full w-3/4 border-l border-border/50 sm:max-w-sm",
					side === "left" &&
						"data-[state=closed]:slide-out-to-left data-[state=open]:slide-in-from-left inset-y-0 left-0 h-full w-3/4 border-r border-border/50 sm:max-w-sm",
					side === "top" &&
						"data-[state=closed]:slide-out-to-top data-[state=open]:slide-in-from-top inset-x-0 top-0 h-auto border-b border-border/50",
					side === "bottom" &&
						"data-[state=closed]:slide-out-to-bottom data-[state=open]:slide-in-from-bottom inset-x-0 bottom-0 h-auto border-t border-border/50",
					className,
				)}
				style={safeAreaStyle}
				{...props}
			>
				{/* Subtle gradient accent */}
				<div className="absolute inset-0 bg-gradient-to-br from-primary/[0.02] via-transparent to-purple-500/[0.02] pointer-events-none" />
				<div className="relative z-10 flex flex-col gap-4 h-full">
					{children}
				</div>
				{/* Absolutely positioned children resolve against the padding box,
				    so `top-4` alone would land inside the safe-area padding — i.e.
				    under the Dynamic Island. Offset by the inset explicitly. */}
				<SheetPrimitive.Close
					style={{ top: `calc(${insetTop} + 1rem)` }}
					className="ring-offset-background focus:ring-ring data-[state=open]:bg-secondary absolute right-4 rounded-xs opacity-70 transition-opacity hover:opacity-100 focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:pointer-events-none z-20"
				>
					<XIcon className="size-4" />
					<span className="sr-only">{t('close', 'Close')}</span>
				</SheetPrimitive.Close>
			</SheetPrimitive.Content>
		</SheetPortal>
	);
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			data-slot="sheet-header"
			className={cn("flex flex-col gap-1.5 p-4", className)}
			{...props}
		/>
	);
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			data-slot="sheet-footer"
			className={cn("mt-auto flex flex-col gap-2 p-4", className)}
			{...props}
		/>
	);
}

function SheetTitle({
	className,
	...props
}: React.ComponentProps<typeof SheetPrimitive.Title>) {
	return (
		<SheetPrimitive.Title
			data-slot="sheet-title"
			className={cn("text-foreground font-semibold", className)}
			{...props}
		/>
	);
}

function SheetDescription({
	className,
	...props
}: React.ComponentProps<typeof SheetPrimitive.Description>) {
	return (
		<SheetPrimitive.Description
			data-slot="sheet-description"
			className={cn("text-muted-foreground text-sm", className)}
			{...props}
		/>
	);
}

export {
	Sheet,
	SheetTrigger,
	SheetClose,
	SheetContent,
	SheetHeader,
	SheetFooter,
	SheetTitle,
	SheetDescription,
};
