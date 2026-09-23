"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BadgeCheck,
	BarChart3,
	BellIcon,
	ChevronsUpDown,
	CreditCard,
	KeyIcon,
	LogInIcon,
	LogOut,
	Server,
	SettingsIcon,
	Sparkles,
	ZapIcon,
} from "lucide-react";
import Link from "next/link";
import { useHub } from "../../hooks/use-hub";
import { userInitials } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { SidebarMenuButton } from "../ui/sidebar";

export interface AccountMenuProps {
	compact?: boolean;
	isMobile?: boolean;
	displayName: string;
	email: string;
	avatar?: string;
	signedIn: boolean;
	notificationCount?: number;
	showUpgrade?: boolean;
	developerMode?: boolean;
	showStatistics?: boolean;
	onSignIn?: () => void | Promise<void>;
	onSignOut?: () => void | Promise<void>;
	onOpenBilling?: () => void | Promise<void>;
}

export function AccountMenu({
	compact = false,
	isMobile = false,
	displayName,
	email,
	avatar,
	signedIn,
	notificationCount = 0,
	showUpgrade = false,
	developerMode = false,
	showStatistics = false,
	onSignIn,
	onSignOut,
	onOpenBilling,
}: Readonly<AccountMenuProps>) {
	const { t } = useTranslation("common");
	const { t: paymentText } = useTranslation("payments");
	const { hub } = useHub();
	const count = notificationCount > 99 ? "99+" : notificationCount;
	const avatarContent = (
		<Avatar className="size-7 shrink-0 rounded-lg ring-1 ring-border/60">
			<AvatarImage src={avatar} alt="" />
			<AvatarFallback className="rounded-lg bg-muted text-[10px] font-semibold">
				{userInitials(displayName, "?")}
			</AvatarFallback>
		</Avatar>
	);
	const identity = (
		<div className="grid min-w-0 flex-1 gap-0.5 text-left leading-tight">
			<span className="truncate text-sm font-medium">{displayName}</span>
			<span className="truncate text-xs text-muted-foreground">{email}</span>
		</div>
	);
	const triggerContent = (
		<>
			<span className="relative shrink-0">
				{avatarContent}
				{notificationCount > 0 ? (
					<span className="absolute -right-1 -top-1 min-w-3.5 rounded-full bg-primary px-1 text-center text-[9px] font-semibold leading-3.5 text-primary-foreground ring-2 ring-sidebar tabular-nums">
						{count}
					</span>
				) : (
					<span
						aria-hidden="true"
						className={cn(
							"absolute -bottom-0.5 -right-0.5 size-2 rounded-full ring-2 ring-sidebar",
							signedIn ? "bg-emerald-500" : "bg-muted-foreground",
						)}
					/>
				)}
			</span>
			{!compact && (
				<>
					<div className="min-w-0 flex-1 group-data-[collapsible=icon]:hidden">
						{identity}
					</div>
					<ChevronsUpDown className="ml-auto size-3.5 shrink-0 text-muted-foreground group-data-[collapsible=icon]:hidden" />
				</>
			)}
		</>
	);
	const itemClass = "min-h-9 gap-2.5 rounded-md px-2.5";

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				{compact ? (
					<button
						type="button"
						aria-label={displayName}
						title={displayName}
						className="relative flex size-9 shrink-0 items-center justify-center rounded-lg transition-colors hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-sidebar-accent"
					>
						{triggerContent}
					</button>
				) : (
					<SidebarMenuButton
						size="lg"
						aria-label={displayName}
						tooltip={displayName}
						className="gap-2.5 overflow-visible rounded-xl border border-sidebar-border/60 bg-sidebar-accent/30 px-2 shadow-xs data-[state=open]:bg-sidebar-accent group-data-[collapsible=icon]:border-transparent group-data-[collapsible=icon]:bg-transparent group-data-[collapsible=icon]:p-0! group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:shadow-none"
					>
						{triggerContent}
					</SidebarMenuButton>
				)}
			</DropdownMenuTrigger>
			<DropdownMenuContent
				side={isMobile ? "bottom" : "right"}
				align="end"
				sideOffset={8}
				className="w-64 max-w-[calc(100vw-1rem)] rounded-xl p-1.5"
			>
				<DropdownMenuLabel className="p-2 font-normal">
					<div className="flex items-center gap-2.5">
						{avatarContent}
						{identity}
					</div>
				</DropdownMenuLabel>
				<DropdownMenuSeparator />
				{signedIn && showUpgrade && (
					<>
						<DropdownMenuItem asChild className={itemClass}>
							<Link href="/subscription">
								<Sparkles className="size-4 text-primary" />
								{t("upgradeToPro", "Upgrade to Pro")}
							</Link>
						</DropdownMenuItem>
						<DropdownMenuSeparator />
					</>
				)}
				<DropdownMenuGroup>
					{signedIn && (
						<>
							<DropdownMenuItem asChild className={itemClass}>
								<Link href="/account">
									<BadgeCheck />
									{t("account", "Account")}
								</Link>
							</DropdownMenuItem>
							<DropdownMenuItem asChild className={itemClass}>
								<Link href="/account/purchases">
									<CreditCard />
									{paymentText("purchases", "Purchases")}
								</Link>
							</DropdownMenuItem>
							{(hub?.payments?.onboarding_enabled ||
								hub?.payments?.servicing_enabled) && (
								<DropdownMenuItem asChild className={itemClass}>
									<Link href="/account/payouts">
										<CreditCard />
										{paymentText("payouts", "Payments and payouts")}
									</Link>
								</DropdownMenuItem>
							)}
							{onOpenBilling && (
								<DropdownMenuItem
									className={itemClass}
									onSelect={() => {
										void onOpenBilling();
									}}
								>
									<CreditCard />
									{t("billing", "Billing")}
								</DropdownMenuItem>
							)}
						</>
					)}
					<DropdownMenuItem asChild className={itemClass}>
						<Link href="/notifications">
							<BellIcon />
							<span className="flex-1">
								{t("notifications", "Notifications")}
							</span>
							{notificationCount > 0 && (
								<span className="rounded-full bg-primary/10 px-1.5 text-[10px] font-semibold text-primary tabular-nums">
									{count}
								</span>
							)}
						</Link>
					</DropdownMenuItem>
					<DropdownMenuItem asChild className={itemClass}>
						<Link href="/settings">
							<SettingsIcon />
							{t("settings", "Settings")}
						</Link>
					</DropdownMenuItem>
					{signedIn && hub?.standalone?.enabled === true && (
						<DropdownMenuItem asChild className={itemClass}>
							<Link href="/settings/devices">
								<Server />
								{t("devices", "Devices")}
							</Link>
						</DropdownMenuItem>
					)}
				</DropdownMenuGroup>
				{signedIn && developerMode && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuGroup>
							<DropdownMenuItem asChild className={itemClass}>
								<Link href="/account/pat">
									<KeyIcon />
									{t("token", "Token")}
								</Link>
							</DropdownMenuItem>
							<DropdownMenuItem asChild className={itemClass}>
								<Link href="/settings/sinks">
									<ZapIcon />
									{t("activeSinks", "Active Sinks")}
								</Link>
							</DropdownMenuItem>
							{showStatistics && (
								<DropdownMenuItem asChild className={itemClass}>
									<Link href="/settings/statistics">
										<BarChart3 />
										{t("boardStatistics", "Board Statistics")}
									</Link>
								</DropdownMenuItem>
							)}
						</DropdownMenuGroup>
					</>
				)}
				{(signedIn ? onSignOut : onSignIn) && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							className={itemClass}
							onSelect={() => {
								void (signedIn ? onSignOut?.() : onSignIn?.());
							}}
						>
							{signedIn ? <LogOut /> : <LogInIcon />}
							{signedIn ? t("logOut", "Log out") : t("logIn", "Log in")}
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
