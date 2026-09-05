"use client";

import { useTranslation } from "@flow-like/locales";
import { LayoutGrid, Package } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useDeveloperMode } from "../../hooks/use-developer-mode";
import { cn } from "../../lib/utils";

const HUB_TABS = [
	{
		key: "apps" as const,
		href: "/store/explore/apps",
		label: "Apps",
		icon: LayoutGrid,
	},
	{
		key: "packages" as const,
		href: "/store/packages",
		label: "Packages",
		icon: Package,
	},
];

export type ExploreHubTab = (typeof HUB_TABS)[number]["key"];

/**
 * Shared header for the Explore hub: community apps and the WASM package
 * registry are one discovery surface with a segmented switch between them.
 */
export function ExploreHubHeader({
	active,
	subtitle,
	className,
	actions,
}: Readonly<{
	active: ExploreHubTab;
	subtitle?: string;
	className?: string;
	actions?: React.ReactNode;
}>) {
	const { t } = useTranslation("store");
	const { developerMode } = useDeveloperMode();
	const tabs = useMemo(
		() => HUB_TABS.filter((tab) => tab.key !== "packages" || developerMode),
		[developerMode],
	);
	return (
		<div
			className={cn(
				"grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1",
				className,
			)}
		>
			<div className="flex min-h-11 min-w-0 items-center">
				<h1 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
					{t("explore", "Explore")}
				</h1>
			</div>
			<div className="flex min-h-11 items-center gap-2">
				{actions}
				{(tabs.length > 1 || active === "packages") && (
					<nav
						aria-label={t("exploreSections", "Explore sections")}
						className="inline-flex items-center rounded-full border border-border/40 bg-muted/30 p-1"
					>
						{tabs.map((tab) => {
							const isActive = tab.key === active;
							return (
								<Link
									key={tab.key}
									href={tab.href}
									aria-current={isActive ? "page" : undefined}
									className={cn(
										"flex min-h-9 items-center gap-1.5 rounded-full px-2.5 py-1.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:px-3.5 sm:text-sm",
										isActive
											? "bg-background text-foreground shadow-sm"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<tab.icon className="h-3.5 w-3.5" />
									{t(tab.key, tab.label)}
								</Link>
							);
						})}
					</nav>
				)}
			</div>
			{subtitle && (
				<p className="col-span-2 min-h-10 text-sm text-muted-foreground sm:min-h-5">
					{subtitle}
				</p>
			)}
		</div>
	);
}
