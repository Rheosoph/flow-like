"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BotIcon,
	EyeIcon,
	MessageSquarePlusIcon,
	PlusIcon,
	SearchIcon,
	StarIcon,
	UsersIcon,
} from "lucide-react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { BlockedPrincipalsButton } from "./blocked-principals";
import type { BotsView } from "./dock-selection";

function ViewSwitch({
	view,
	onChange,
}: Readonly<{ view: BotsView; onChange: (view: BotsView) => void }>) {
	const { t } = useTranslation("bots");
	const options: { value: BotsView; label: string }[] = [
		{ value: "chats", label: t("teams.chats.view.chats", "Chats") },
		{ value: "org", label: t("teams.chats.view.org", "Org chart") },
	];
	return (
		<fieldset className="m-0 inline-flex min-w-0 rounded-[10px] border bg-card p-0.5">
			<legend className="sr-only">
				{t("teams.chats.view.label", "Bots view")}
			</legend>
			{options.map((option) => (
				<button
					key={option.value}
					type="button"
					aria-pressed={view === option.value}
					onClick={() => onChange(option.value)}
					className={cn(
						"rounded-[8px] px-3 py-1.5 text-sm font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
						view === option.value
							? "bg-primary text-primary-foreground"
							: "text-muted-foreground hover:text-foreground",
					)}
				>
					{option.label}
				</button>
			))}
		</fieldset>
	);
}

function IconAction({
	label,
	icon,
	onClick,
	primary = false,
}: Readonly<{
	label: string;
	icon: React.ReactNode;
	onClick?: () => void;
	primary?: boolean;
}>) {
	return (
		<Button
			type="button"
			variant={primary ? "default" : "outline"}
			className="h-10 shrink-0 rounded-[10px] px-3 max-lg:size-10 max-lg:px-0"
			aria-label={label}
			title={label}
			disabled={!onClick}
			onClick={onClick}
		>
			{icon}
			<span className="max-lg:hidden">{label}</span>
		</Button>
	);
}

/** Bots tab header: the Chats / Org chart switch, search and the ways to start something. */
export function BotsChatsHeader({
	view,
	onViewChange,
	showViewSwitch,
	search,
	onSearchChange,
	onNewBot,
	onNewRoom,
	onNewMessage,
	onShowStarred,
	onManageVisibility,
	canManageBlocks = false,
}: Readonly<{
	view: BotsView;
	onViewChange: (view: BotsView) => void;
	showViewSwitch: boolean;
	search: string;
	onSearchChange: (value: string) => void;
	onNewBot?: () => void;
	onNewRoom?: () => void;
	onNewMessage?: () => void;
	onShowStarred?: () => void;
	onManageVisibility?: () => void;
	canManageBlocks?: boolean;
}>) {
	const { t } = useTranslation("bots");
	return (
		<header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-4 py-3">
			<div className="flex items-center gap-3">
				<span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-muted-foreground">
					<BotIcon className="size-3.75" aria-hidden />
					{t("crew.eyebrow", "Bots")}
				</span>
				{showViewSwitch && <ViewSwitch view={view} onChange={onViewChange} />}
			</div>
			<div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-2">
				{view === "chats" && (
					<label className="flex h-10 min-w-0 flex-1 basis-48 items-center gap-2 rounded-[10px] border bg-card px-3 text-muted-foreground focus-within:border-primary sm:max-w-[270px]">
						<SearchIcon className="size-4 shrink-0" aria-hidden />
						<input
							type="search"
							value={search}
							onChange={(event) => onSearchChange(event.target.value)}
							aria-label={t(
								"crew.search.label",
								"Search bots, people and chats",
							)}
							placeholder={t(
								"crew.search.placeholder",
								"Search bots, people, chats",
							)}
							className="h-9 min-w-0 flex-1 appearance-none bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
						/>
					</label>
				)}
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
				{onShowStarred && (
					<IconAction
						label={t("crew.marks.starredShort", "Starred")}
						icon={<StarIcon aria-hidden />}
						onClick={onShowStarred}
					/>
				)}
				<IconAction
					label={t("crew.newMessage", "New message")}
					icon={<MessageSquarePlusIcon aria-hidden />}
					onClick={onNewMessage}
				/>
				<IconAction
					label={t("crew.newRoom", "New room")}
					icon={<UsersIcon aria-hidden />}
					onClick={onNewRoom}
				/>
				<IconAction
					primary
					label={t("crew.newBot", "New bot")}
					icon={<PlusIcon aria-hidden />}
					onClick={onNewBot}
				/>
			</div>
		</header>
	);
}
