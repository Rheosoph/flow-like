"use client";

import { cn, useSidebar } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import {
	CompassIcon,
	HomeIcon,
	LayoutGridIcon,
	type LucideIcon,
	MenuIcon,
	SparklesIcon,
} from "lucide-react";
import Link from "next/link";
import { usePathname } from "next/navigation";

interface NavDestination {
	title: string;
	href: string;
	icon: LucideIcon;
	isActive: (pathname: string) => boolean;
}

// Home / Explore sit left of the emphasised FlowPilot centre; My Apps sits right.
const DESTINATIONS: readonly NavDestination[] = [
	{ title: "Home", href: "/", icon: HomeIcon, isActive: (p) => p === "/" },
	{
		title: "Explore",
		href: "/store/explore/apps",
		icon: CompassIcon,
		isActive: (p) => p.startsWith("/store"),
	},
	{
		title: "My Apps",
		href: "/library",
		icon: LayoutGridIcon,
		isActive: (p) => p.startsWith("/library"),
	},
] as const;

function NavCell({
	dest,
	pathname,
}: Readonly<{ dest: NavDestination; pathname: string }>) {
	const active = dest.isActive(pathname);
	const Icon = dest.icon;
	return (
		<Link
			href={dest.href}
			aria-label={dest.title}
			aria-current={active ? "page" : undefined}
			className={cn(
				"flex flex-1 flex-col items-center justify-center gap-0.5 text-[10.5px] font-medium outline-none transition-colors focus-visible:text-primary",
				active ? "text-primary" : "text-muted-foreground hover:text-foreground",
			)}
		>
			<Icon className="size-5" strokeWidth={active ? 2.4 : 2} aria-hidden />
			<span className="leading-none">{dest.title}</span>
		</Link>
	);
}

/**
 * Persistent thumb-reachable navigation on phones. Rendered in-flow at the
 * bottom of the app shell (so page content never hides behind it) with FlowPilot
 * emphasised as the primary mobile surface. "Menu" opens the full sidebar sheet
 * for everything else (profiles, settings, university, …).
 */
export function MobileBottomNav() {
	const { t } = useTranslation("common");
	const pathname = usePathname();
	const { toggleSidebar } = useSidebar();
	const flowpilotActive = pathname === "/chat" || pathname.startsWith("/chat/");

	return (
		<nav
			aria-label="Primary"
			className="fl-mobile-bottom-nav md:hidden shrink-0 border-t border-border/60 bg-card/90 backdrop-blur-fallback pb-safe px-safe"
		>
			<div className="mx-auto flex h-14 max-w-xl items-stretch justify-around px-1">
				<NavCell dest={DESTINATIONS[0]} pathname={pathname} />
				<NavCell dest={DESTINATIONS[1]} pathname={pathname} />

				<Link
					href="/chat"
					aria-label={t("flowpilot", "FlowPilot")}
					aria-current={flowpilotActive ? "page" : undefined}
					className="flex flex-1 flex-col items-center justify-center gap-0.5 outline-none"
				>
					<span
						className={cn(
							"-mt-1 grid size-9 place-items-center rounded-full text-white shadow-md shadow-primary/30",
							"bg-linear-to-br from-primary via-primary to-purple-600",
							flowpilotActive &&
								"ring-2 ring-primary/40 ring-offset-1 ring-offset-background",
						)}
					>
						<SparklesIcon className="size-5" strokeWidth={2.2} aria-hidden />
					</span>
					<span
						className={cn(
							"text-[10.5px] font-medium leading-none",
							flowpilotActive ? "text-primary" : "text-muted-foreground",
						)}
					>
						{t("flowpilot", "FlowPilot")}
					</span>
				</Link>

				<NavCell dest={DESTINATIONS[2]} pathname={pathname} />

				<button
					type="button"
					onClick={toggleSidebar}
					aria-label={t("openMenu", "Open menu")}
					className="flex flex-1 flex-col items-center justify-center gap-0.5 text-[10.5px] font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:text-primary"
				>
					<MenuIcon className="size-5" strokeWidth={2} aria-hidden />
					<span className="leading-none">{t("menu", "Menu")}</span>
				</button>
			</div>
		</nav>
	);
}
