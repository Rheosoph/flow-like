"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { Check, Crown, Loader2, Mail, Sparkles, Zap } from "lucide-react";
import { useMemo } from "react";
import { cn } from "../../lib/utils";
import type { ITierInfo } from "../../state/backend-state/user-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";

export const TIER_ORDER = ["FREE", "PREMIUM", "PRO", "ENTERPRISE"];

export const TIER_COLORS: Record<string, string> = {
	FREE: "bg-muted text-foreground",
	PREMIUM: "bg-linear-to-br from-amber-500 to-orange-600 text-white",
	PRO: "bg-linear-to-br from-violet-500 to-purple-600 text-white",
	ENTERPRISE: "bg-linear-to-br from-blue-500 to-indigo-600 text-white",
};

export const TIER_ICONS: Record<string, React.ReactNode> = {
	FREE: <Zap className="h-5 w-5" />,
	PREMIUM: <Sparkles className="h-5 w-5" />,
	PRO: <Crown className="h-5 w-5" />,
	ENTERPRISE: <Crown className="h-5 w-5" />,
};

export const ENTERPRISE_TIER = "ENTERPRISE";

export function formatBytes(bytes: number): string {
	if (bytes === 0) return "0 B";
	const k = 1024;
	const sizes = ["B", "KB", "MB", "GB", "TB"];
	const i = Math.floor(Math.log(bytes) / Math.log(k));
	return `${Number.parseFloat((bytes / k ** i).toFixed(2))} ${sizes[i]}`;
}

export function formatPrice(
	amount: number,
	currency: string,
	interval?: string,
): string {
	const formatter = new Intl.NumberFormat("en-US", {
		style: "currency",
		currency: currency.toUpperCase(),
		minimumFractionDigits: amount % 100 === 0 ? 0 : 2,
	});
	const formatted = formatter.format(amount / 100);
	return interval ? `${formatted}/${interval}` : formatted;
}

/** Fallback bullets derived from the tier limits when the hub config ships no curated feature list. */
export function deriveTierFeatures(tier: ITierInfo): string[] {
	const items: string[] = [];
	if (tier.max_non_visible_projects < 0) {
		items.push("Unlimited online projects");
	} else if (tier.max_non_visible_projects > 0) {
		items.push(`${tier.max_non_visible_projects} online projects`);
	}
	if (tier.max_remote_executions < 0) {
		items.push("Unlimited cloud runs");
	} else if (tier.max_remote_executions > 0) {
		items.push(
			i18next.t("valCloudRunsmonth", "{{val}} cloud runs/month", {
				val: tier.max_remote_executions.toLocaleString("en-US"),
			}),
		);
	}
	if (tier.max_total_size < 0) {
		items.push("Unlimited cloud storage");
	} else if (tier.max_total_size > 0) {
		items.push(`${formatBytes(tier.max_total_size)} cloud storage`);
	}
	if (tier.max_llm_cost < 0) {
		items.push("Unlimited AI credits");
	} else if (tier.max_llm_cost > 0) {
		items.push(`$${(tier.max_llm_cost / 100).toFixed(2)} AI credits per month`);
	}
	if (tier.llm_tiers.length > 0) {
		items.push(
			i18next.t("accessToValModels", "Access to {{val}} models", {
				val: tier.llm_tiers.map((t) => t.toLowerCase()).join(", "),
			}),
		);
	}
	return items;
}

export interface TierCardProps {
	tierKey: string;
	tier: ITierInfo;
	currentTier: string;
	onUpgrade: (tier: string) => Promise<void> | void;
	onManageBilling: () => Promise<void> | void;
	isLoading?: boolean;
	/** Tighter paddings for use inside the upgrade dialog. */
	compact?: boolean;
	/** Overrides the config highlight, e.g. when a specific tier unlocks the blocked action. */
	emphasize?: boolean;
}

/**
 * A single pricing tier. Marketing metadata (display name, tagline, curated
 * feature bullets, highlight badge) comes from the hub's `conversion.tier_display`
 * config; without it the card falls back to limits-derived bullets.
 */
export function TierCard({
	tierKey,
	tier,
	currentTier,
	onUpgrade,
	onManageBilling,
	isLoading = false,
	compact = false,
	emphasize,
}: Readonly<TierCardProps>) {
	const { t } = useTranslation("common");
	const features = useMemo(
		() => (tier.features?.length ? tier.features : deriveTierFeatures(tier)),
		[tier],
	);

	const isCurrentTier = currentTier === tierKey;
	const isEnterprise = tierKey === ENTERPRISE_TIER;
	const isPaid = tierKey !== "FREE" && (tier.product_id || isEnterprise);
	const hasExistingSubscription =
		currentTier !== "FREE" && currentTier !== ENTERPRISE_TIER;
	// Enterprise accounts are managed outside Stripe — never route them into a
	// fresh checkout or the (nonexistent) billing portal.
	const isEnterpriseCustomer = currentTier === ENTERPRISE_TIER;
	const highlighted = emphasize ?? (tier.highlight && !isCurrentTier);
	const displayName = tier.display_name ?? tier.name ?? tierKey;
	const badgeLabel = tier.badge ?? "Most popular";
	const colorClass = TIER_COLORS[tierKey] ?? TIER_COLORS.FREE;
	const icon = TIER_ICONS[tierKey] ?? TIER_ICONS.FREE;

	return (
		<div
			className={cn(
				"relative flex h-full flex-col rounded-xl border bg-card transition-shadow",
				compact ? "p-5" : "p-6",
				highlighted
					? "border-primary shadow-floating ring-1 ring-primary/30"
					: "border-border dark:border-white/15",
				isCurrentTier && "border-primary/40",
			)}
		>
			{highlighted && (
				<Badge className="absolute -top-2.5 left-1/2 -translate-x-1/2 bg-primary text-primary-foreground shadow-md">
					{badgeLabel}
				</Badge>
			)}
			{isCurrentTier && !highlighted && (
				<Badge
					variant="outline"
					className="absolute -top-2.5 left-1/2 -translate-x-1/2 bg-card"
				>
					{t("currentPlan", "Current plan")}
				</Badge>
			)}

			<div className="flex items-center gap-3">
				<div
					className={cn(
						"flex h-10 w-10 shrink-0 items-center justify-center rounded-lg",
						colorClass,
					)}
				>
					{icon}
				</div>
				<div className="min-w-0">
					<h3 className="truncate text-lg font-semibold leading-tight">
						{displayName}
					</h3>
					{tier.tagline && (
						<p className="truncate text-xs text-muted-foreground">
							{tier.tagline}
						</p>
					)}
				</div>
			</div>

			<div
				className={cn("flex items-baseline gap-1", compact ? "mt-4" : "mt-5")}
			>
				{isEnterprise ? (
					<span className="text-3xl font-bold tracking-tight">
						{t("custom", "Custom")}
					</span>
				) : tier.price ? (
					<>
						<span className="text-3xl font-bold tracking-tight">
							{formatPrice(tier.price.amount, tier.price.currency)}
						</span>
						{tier.price.interval && (
							<span className="text-sm text-muted-foreground">
								/{tier.price.interval}
							</span>
						)}
					</>
				) : isPaid ? (
					<span className="text-3xl font-bold tracking-tight text-muted-foreground">
						—
					</span>
				) : (
					<span className="text-3xl font-bold tracking-tight">
						{t("free", "Free")}
					</span>
				)}
			</div>

			<ul className={cn("flex-1 space-y-2", compact ? "mt-4" : "mt-6")}>
				{features.map((feature) => (
					<li key={feature} className="flex items-start gap-2">
						<Check className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
						<span className="text-sm text-muted-foreground">{feature}</span>
					</li>
				))}
			</ul>

			<div className={compact ? "mt-5" : "mt-6"}>
				{isCurrentTier ? (
					<Button className="w-full" variant="outline" disabled>
						{t("currentPlan", "Current plan")}
					</Button>
				) : isEnterprise ? (
					<Button className="w-full" variant="outline" asChild>
						<a
							href={tier.contact_url ?? "#"}
							target="_blank"
							rel="noreferrer external"
						>
							<Mail className="h-4 w-4" />
							{t("talkToSales", "Talk to sales")}
						</a>
					</Button>
				) : isEnterpriseCustomer ? (
					<Button className="w-full" variant="outline" disabled>
						{t("managedByYourAgreement", "Managed by your agreement")}
					</Button>
				) : isPaid ? (
					<Button
						className="w-full"
						variant={highlighted ? "default" : "outline"}
						onClick={() =>
							hasExistingSubscription ? onManageBilling() : onUpgrade(tierKey)
						}
						disabled={isLoading}
					>
						{isLoading ? (
							<>
								<Loader2 className="h-4 w-4 animate-spin" />
								Processing...
							</>
						) : hasExistingSubscription ? (
							"Change plan"
						) : (
							t("upgradeToDisplayname", "Upgrade to {{displayName}}", {
								displayName,
							})
						)}
					</Button>
				) : tierKey === "FREE" ? (
					<Button className="w-full" variant="outline" disabled>
						{t("included", "Included")}
					</Button>
				) : (
					<Button className="w-full" variant="outline" disabled>
						{t("notAvailable", "Not available")}
					</Button>
				)}
			</div>
		</div>
	);
}
