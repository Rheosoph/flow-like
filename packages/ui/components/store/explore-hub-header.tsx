"use client";

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
	const { developerMode } = useDeveloperMode();
	const tabs = useMemo(
		() => HUB_TABS.filter((tab) => tab.key !== "packages" || developerMode),
		[developerMode],
	);
	return (
		<div
			className={cn(
				"flex flex-wrap items-end justify-between gap-3",
				className,
			)}
		>
			<div className="space-y-1 min-w-0">
				<h1 className="text-2xl font-bold tracking-tight text-foreground">
					Explore
				</h1>
				{subtitle && (
					<p className="text-sm text-muted-foreground">{subtitle}</p>
				)}
			</div>
			<div className="flex items-center gap-2">
				{actions}
				<nav
					aria-label="Explore sections"
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
									"flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors",
									isActive
										? "bg-background text-foreground shadow-sm"
										: "text-muted-foreground hover:text-foreground",
								)}
							>
								<tab.icon className="h-3.5 w-3.5" />
								{tab.label}
							</Link>
						);
					})}
				</nav>
			</div>
		</div>
	);
}
