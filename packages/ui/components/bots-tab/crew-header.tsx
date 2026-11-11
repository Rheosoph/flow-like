"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BotIcon,
	EyeIcon,
	MessageSquareIcon,
	PlusIcon,
	SearchIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../ui/button";
import { BlockedPrincipalsButton } from "./blocked-principals";

function useHour() {
	const [hour, setHour] = useState<number | null>(null);
	useEffect(() => {
		const update = () => setHour(new Date().getHours());
		update();
		const timer = setInterval(update, 60_000);
		return () => clearInterval(timer);
	}, []);
	return hour ?? 12;
}

function useGreeting(name?: string) {
	const { t } = useTranslation("bots");
	const hour = useHour();
	if (hour < 12) {
		return name
			? t("crew.greeting.morning", "Good morning, {{name}}", { name })
			: t("crew.greeting.morningAnonymous", "Good morning");
	}
	if (hour < 18) {
		return name
			? t("crew.greeting.afternoon", "Good afternoon, {{name}}", { name })
			: t("crew.greeting.afternoonAnonymous", "Good afternoon");
	}
	return name
		? t("crew.greeting.evening", "Good evening, {{name}}", { name })
		: t("crew.greeting.eveningAnonymous", "Good evening");
}

/**
 * `onNewBot` is left out while bots can't be created; `onShowChat` only while the dock is closed;
 * `onManageVisibility` where no profile can hide bots.
 */
export function CrewHeader({
	name,
	summary,
	search,
	onSearchChange,
	onNewBot,
	onShowChat,
	onManageVisibility,
	canManageBlocks = false,
}: Readonly<{
	name?: string;
	summary: string;
	search: string;
	onSearchChange: (value: string) => void;
	onNewBot?: () => void;
	onShowChat?: () => void;
	onManageVisibility?: () => void;
	canManageBlocks?: boolean;
}>) {
	const { t } = useTranslation("bots");
	const greeting = useGreeting(name);
	return (
		<header className="flex flex-wrap items-end justify-between gap-4 @max-[520px]/crew:gap-3">
			<div className="flex min-w-0 flex-col gap-1">
				<span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-muted-foreground @max-[520px]/crew:hidden">
					<BotIcon className="size-3.75" aria-hidden />
					{t("crew.eyebrow", "Bots")}
				</span>
				<h1 className="text-[28px] font-semibold leading-tight tracking-tight @max-[520px]/crew:text-lg">
					{greeting}
				</h1>
				<p className="text-sm text-muted-foreground @max-[520px]/crew:hidden">
					{summary}
				</p>
			</div>
			<div className="flex flex-wrap items-center gap-2 @max-[520px]/crew:w-full @max-[520px]/crew:flex-nowrap">
				{canManageBlocks && <BlockedPrincipalsButton />}
				{onManageVisibility && (
					<Button
						type="button"
						variant="outline"
						size="icon"
						className="size-10 shrink-0 rounded-[10px]"
						onClick={onManageVisibility}
						aria-label={t("crew.visibility.title", "Bots in this profile")}
						title={t("crew.visibility.title", "Bots in this profile")}
					>
						<EyeIcon aria-hidden />
					</Button>
				)}
				{onShowChat && (
					<Button
						type="button"
						variant="outline"
						className="h-10 rounded-[10px]"
						onClick={onShowChat}
					>
						<MessageSquareIcon aria-hidden />
						{t("crew.showChat", "Show chat")}
					</Button>
				)}
				<label className="flex h-10 w-[270px] max-w-full items-center gap-2 rounded-[10px] border bg-card px-3 text-muted-foreground focus-within:border-primary @max-[520px]/crew:w-auto @max-[520px]/crew:min-w-0 @max-[520px]/crew:flex-1">
					<SearchIcon className="size-4 shrink-0" aria-hidden />
					<input
						type="search"
						value={search}
						onChange={(event) => onSearchChange(event.target.value)}
						aria-label={t("crew.search.label", "Search bots, people and chats")}
						placeholder={t(
							"crew.search.placeholder",
							"Search bots, people, chats",
						)}
						className="h-9 min-w-0 flex-1 appearance-none bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
					/>
				</label>
				<Button
					type="button"
					className="h-10 rounded-[10px] px-3.5 font-semibold @max-[520px]/crew:size-10 @max-[520px]/crew:shrink-0 @max-[520px]/crew:px-0"
					aria-label={t("crew.newBot", "New bot")}
					title={t("crew.newBot", "New bot")}
					disabled={!onNewBot}
					onClick={onNewBot}
				>
					<PlusIcon aria-hidden />
					<span className="@max-[520px]/crew:hidden">
						{t("crew.newBot", "New bot")}
					</span>
				</Button>
			</div>
		</header>
	);
}
