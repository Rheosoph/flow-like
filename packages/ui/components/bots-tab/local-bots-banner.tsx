"use client";

import { useTranslation } from "@flow-like/locales";
import { HardDriveIcon, XIcon } from "lucide-react";
import { useCallback, useState } from "react";
import type { IBot } from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import {
	isBannerHidden,
	readBannerHidden,
	writeBannerHidden,
} from "./local-bots-banner-model";

/**
 * Whether the banner shows for an account's bots on this computer. Hiding it lasts across
 * visits, until a bot is made here afterwards.
 */
export function useLocalBotsBanner(
	account: string | null,
	bots: readonly IBot[],
) {
	const [hidden, setHidden] = useState(readBannerHidden);
	const hide = useCallback(() => {
		if (!account) return;
		const next = { account, at: Date.now() };
		setHidden(next);
		writeBannerHidden(next);
	}, [account]);
	return {
		shown:
			account !== null &&
			bots.length > 0 &&
			!isBannerHidden(hidden, account, bots),
		hide,
	};
}

/** Signed in with bots that still live only on this computer: offers the move review. */
export function LocalBotsBanner({
	count,
	onReview,
	onDismiss,
}: Readonly<{ count: number; onReview: () => void; onDismiss: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<section
			aria-label={t("move.banner.label", "Bots on this computer")}
			className="relative flex items-center gap-3 rounded-2xl border bg-card px-4 py-3 @max-[560px]/crew:flex-col @max-[560px]/crew:items-stretch"
		>
			<div className="grid min-w-0 flex-1 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0.5 @max-[560px]/crew:gap-y-2">
				<span className="row-span-2 flex size-9 items-center justify-center rounded-full bg-muted text-muted-foreground @max-[560px]/crew:row-span-1">
					<HardDriveIcon className="size-4" aria-hidden />
				</span>
				<p className="text-sm font-semibold @max-[560px]/crew:pr-8">
					{t("move.banner.title", "{{count}} bots live only on this computer", {
						count,
						defaultValue_one: "{{count}} bot lives only on this computer",
					})}
				</p>
				<p className="col-start-2 text-xs text-muted-foreground @max-[560px]/crew:col-span-full">
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
				className="shrink-0 rounded-[9px] @max-[560px]/crew:self-start"
				onClick={onReview}
			>
				{t("move.banner.action", "Review and move")}
			</Button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="-mr-1 size-8 shrink-0 text-muted-foreground @max-[560px]/crew:absolute @max-[560px]/crew:top-3.5 @max-[560px]/crew:right-3 @max-[560px]/crew:mr-0"
				onClick={onDismiss}
				aria-label={t("move.banner.dismiss", "Hide")}
				title={t("move.banner.dismiss", "Hide")}
			>
				<XIcon aria-hidden />
			</Button>
		</section>
	);
}
