"use client";

import { useTranslation } from "@flow-like/locales";
import { HardDriveIcon } from "lucide-react";
import { Button } from "../ui/button";

/** Signed in with bots that still live only on this computer: offers the move review. */
export function LocalBotsBanner({
	count,
	onReview,
}: Readonly<{ count: number; onReview: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<section
			aria-label={t("move.banner.label", "Bots on this computer")}
			className="flex flex-wrap items-center gap-3 rounded-2xl border bg-card px-4 py-3"
		>
			<span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
				<HardDriveIcon className="size-4" aria-hidden />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-0.5">
				<p className="text-sm font-semibold">
					{t("move.banner.title", "{{count}} bots live only on this computer", {
						count,
						defaultValue_one: "{{count}} bot lives only on this computer",
					})}
				</p>
				<p className="text-xs text-muted-foreground">
					{t(
						"move.banner.description",
						"These bots aren't connected to your account, so your account's bots can't find or message them. They'll still run in the desktop app after you move them.",
					)}
				</p>
			</div>
			<Button
				type="button"
				variant="outline"
				size="sm"
				className="rounded-[9px]"
				onClick={onReview}
			>
				{t("move.banner.action", "Review and move")}
			</Button>
		</section>
	);
}
