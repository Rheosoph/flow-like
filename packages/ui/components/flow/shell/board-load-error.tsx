"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, RotateCwIcon, TriangleAlertIcon } from "lucide-react";
import Link from "next/link";
import { memo } from "react";
import { isMissingResourceError } from "../../../lib/api-error";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";

/**
 * Replaces the whole editor when the board could not be loaded. The rail,
 * sidebars and panels all describe a board, so without one they would only
 * show empty views beside a broken canvas.
 */
export const BoardLoadError = memo(function BoardLoadError({
	error,
	retrying,
	onRetry,
	back,
}: Readonly<{
	error: unknown;
	retrying: boolean;
	onRetry: () => void;
	/** Omitted where the host owns navigation. */
	back?: { href: string; label: string };
}>) {
	const { t } = useTranslation("flow");
	const missing = isMissingResourceError(error);
	const detail = !missing && error instanceof Error ? error.message : undefined;

	return (
		<div
			role="alert"
			className="flex h-full w-full items-center justify-center bg-background p-6"
		>
			<div className="flex max-w-md flex-col items-center gap-3 text-center">
				<TriangleAlertIcon className="size-8 text-muted-foreground" />
				<h2 className="text-base font-medium">
					{missing
						? t("boardNotFoundTitle", "This flow no longer exists")
						: t("boardLoadFailed", "The flow could not be loaded.")}
				</h2>
				<p className="text-sm text-muted-foreground">
					{missing
						? t(
								"boardNotFoundDescription",
								"It may have been deleted. Open another flow from the app's flow list.",
							)
						: t(
								"boardLoadFailedDescription",
								"Check your connection and try again. If it keeps failing, go back to the app's flows.",
							)}
				</p>
				{detail && (
					<p className="max-w-full select-text break-words rounded-md bg-muted px-2 py-1 font-mono text-xs text-muted-foreground">
						{detail}
					</p>
				)}
				<div className="mt-2 flex flex-wrap justify-center gap-2">
					{back && (
						<Button asChild size="sm" variant={missing ? "default" : "outline"}>
							<Link href={back.href}>
								<ArrowLeftIcon />
								{back.label}
							</Link>
						</Button>
					)}
					{!missing && (
						<Button size="sm" onClick={onRetry} disabled={retrying}>
							<RotateCwIcon className={cn(retrying && "animate-spin")} />
							{t("retry", "Retry")}
						</Button>
					)}
				</div>
			</div>
		</div>
	);
});
