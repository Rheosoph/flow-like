"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, LockIcon, XIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "../ui/button";

/** Top of the dock. Below the dock breakpoint it fills the view, so "back" replaces "close". */
export function DockHeader({
	avatar,
	title,
	subtitle,
	encrypted = false,
	status,
	actions,
	onClose,
}: Readonly<{
	avatar?: ReactNode;
	title: ReactNode;
	subtitle?: ReactNode;
	encrypted?: boolean;
	status?: ReactNode;
	actions?: ReactNode;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const encryptedLabel = t("crew.encrypted", "End-to-end encrypted");
	return (
		<header className="flex min-h-17 shrink-0 items-center gap-3 border-b py-3 pr-3 pl-4.5 @max-[820px]/bots:pl-2">
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
			{avatar}
			<div className="min-w-0 flex-1">
				<div className="flex min-w-0 items-center gap-1.5">
					<h2 className="flex min-w-0 items-center gap-1.5 text-[15px] font-semibold">
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
					<p className="mt-0.5 truncate text-xs text-muted-foreground">
						{subtitle}
					</p>
				)}
			</div>
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
