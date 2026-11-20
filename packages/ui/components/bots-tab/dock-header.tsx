"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, LockIcon, XIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";

const IDENTITY_CLASS =
	"flex min-w-0 flex-1 items-center gap-3 @max-[820px]/bots:gap-2.5";

/**
 * Top of the dock. Below the dock breakpoint it fills the view, so "back" replaces "close". With
 * `onOpenDetails` the avatar and title open who or what the conversation is with.
 */
export function DockHeader({
	avatar,
	title,
	subtitle,
	encrypted = false,
	status,
	actions,
	detailsLabel,
	onOpenDetails,
	onClose,
}: Readonly<{
	avatar?: ReactNode;
	title: ReactNode;
	subtitle?: ReactNode;
	encrypted?: boolean;
	status?: ReactNode;
	actions?: ReactNode;
	detailsLabel?: string;
	onOpenDetails?: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const encryptedLabel = t("crew.encrypted", "End-to-end encrypted");
	const identity = (
		<>
			{avatar}
			<div className="min-w-0 flex-1 text-left">
				<div className="flex min-w-0 items-center gap-1.5">
					<h2 className="flex min-w-0 items-center gap-1.5 text-[15px] font-semibold @max-[820px]/bots:leading-tight">
						<span className="truncate">{title}</span>
						{encrypted && (
							<LockIcon
								className="size-3.5 shrink-0 text-muted-foreground"
								role="img"
								aria-label={encryptedLabel}
							/>
						)}
					</h2>
					{status}
				</div>
				{subtitle && (
					<p className="mt-0.5 truncate text-xs text-muted-foreground @max-[820px]/bots:mt-0 @max-[820px]/bots:text-[11.5px]">
						{subtitle}
					</p>
				)}
			</div>
		</>
	);
	return (
		<header className="flex min-h-17 shrink-0 items-center gap-3 border-b py-3 pr-3 pl-4.5 @max-[820px]/bots:min-h-12 @max-[820px]/bots:gap-0 @max-[820px]/bots:py-1.5 @max-[820px]/bots:pr-1 @max-[820px]/bots:pl-0.5">
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="hidden shrink-0 text-muted-foreground @max-[820px]/bots:inline-flex"
				onClick={onClose}
				aria-label={t("crew.dock.back", "Back to your crew")}
				title={t("crew.dock.back", "Back to your crew")}
			>
				<ArrowLeftIcon aria-hidden />
			</Button>
			{onOpenDetails ? (
				<button
					type="button"
					onClick={onOpenDetails}
					aria-label={detailsLabel}
					title={detailsLabel}
					className={cn(
						IDENTITY_CLASS,
						"-my-1 touch-manipulation rounded-xl py-1 pr-2 pl-1 outline-none transition-[background-color,transform] duration-100 hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.98] active:bg-muted",
					)}
				>
					{identity}
				</button>
			) : (
				<div className={IDENTITY_CLASS}>{identity}</div>
			)}
			{actions}
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="shrink-0 text-muted-foreground @max-[820px]/bots:hidden"
				onClick={onClose}
				aria-label={t("crew.dock.close", "Close chat")}
				title={t("crew.dock.close", "Close chat")}
			>
				<XIcon aria-hidden />
			</Button>
		</header>
	);
}
