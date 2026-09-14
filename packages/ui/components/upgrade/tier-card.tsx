"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { ChevronDown, Crown, Loader2, Mail, Sparkles, Zap } from "lucide-react";
import { formatQuota } from "../../lib/quota";
import { estimateCloudRuns } from "../../lib/runtime-estimate";
import { cn } from "../../lib/utils";
import type { ITierInfo } from "../../state/backend-state/user-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";

export const TIER_ORDER = ["FREE", "PREMIUM", "PRO", "MAX", "ENTERPRISE"];

export const TIER_COLORS: Record<string, string> = {
	FREE: "bg-muted text-foreground",
	PREMIUM: "bg-linear-to-br from-amber-500 to-orange-600 text-white",
	PRO: "bg-linear-to-br from-violet-500 to-purple-600 text-white",
	MAX: "bg-linear-to-br from-rose-500 to-orange-600 text-white",
	ENTERPRISE: "bg-linear-to-br from-blue-500 to-indigo-600 text-white",
};

export const TIER_ICONS: Record<string, React.ReactNode> = {
	FREE: <Zap className="h-5 w-5" />,
	PREMIUM: <Sparkles className="h-5 w-5" />,
	PRO: <Crown className="h-5 w-5" />,
	MAX: <Crown className="h-5 w-5" />,
	ENTERPRISE: <Crown className="h-5 w-5" />,
};

export const ENTERPRISE_TIER = "ENTERPRISE";

export function formatBytes(bytes: number): string {
	if (bytes < 0) return "Unlimited";
	if (bytes === 0) return "0 B";
	const k = 1000;
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
		items.push("Unlimited private cloud projects");
	} else if (tier.max_non_visible_projects > 0) {
		items.push(`${tier.max_non_visible_projects} private cloud projects`);
	}
	if (tier.max_runtime_ms !== undefined && tier.max_runtime_ms >= 0) {
		items.push(
			`${tier.max_runtime_ms / 60_000 < 60 ? `${tier.max_runtime_ms / 60_000} minutes` : `${tier.max_runtime_ms / 3_600_000} hours`} of cloud runtime per month`,
		);
	}
	if (tier.max_remote_executions >= 0)
		items.push(
			`Up to ${tier.max_remote_executions.toLocaleString()} cloud starts per month`,
		);
	if (tier.max_total_size < 0) {
		items.push("Unlimited cloud storage");
	} else if (tier.max_total_size > 0) {
		items.push(`${formatBytes(tier.max_total_size)} cloud storage`);
	}
	const aiMicros = tier.max_ai_cost_micros ?? tier.max_llm_cost * 10_000;
	if (aiMicros >= 0)
		items.push(`€${aiMicros / 1_000_000} of hosted AI usage per month`);
	if (tier.max_llm_calls !== undefined && tier.max_llm_calls >= 0)
		items.push(
			`Up to ${tier.max_llm_calls.toLocaleString()} hosted AI operations per month`,
		);
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
	onUpgrade: (
		tier: string,
		priceId?: string,
		interval?: "month" | "year",
	) => Promise<void> | void;
	onManageBilling: () => Promise<void> | void;
	isLoading?: boolean;
	/** Tighter paddings for use inside the upgrade dialog. */
	compact?: boolean;
	/** Overrides the config highlight, e.g. when a specific tier unlocks the blocked action. */
	emphasize?: boolean;
	/** Show a collapsed disclosure containing the remaining plan limits. */
	showDetails?: boolean;
	/** Emphasize the allowance involved in the current plan limit. */
	focusResource?: string;
	runtimeAverageMs?: number;
}

/**
 * A plan summary with runtime, hosted AI allowance and storage. Further limits
 * stay available in a disclosure, with checkout using the eligible Stripe price.
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
	showDetails = true,
	focusResource,
	runtimeAverageMs,
}: Readonly<TierCardProps>) {
	const { t } = useTranslation("common");
	const runEstimate =
		runtimeAverageMs === undefined
			? null
			: estimateCloudRuns(
					tier.max_runtime_ms,
					tier.max_remote_executions,
					runtimeAverageMs,
				);

	const isCurrentTier = currentTier === tierKey;
	const isEnterprise = tierKey === ENTERPRISE_TIER;
	const isPaid = tierKey !== "FREE" && (tier.product_id || isEnterprise);
	const hasExistingSubscription =
		currentTier !== "FREE" && currentTier !== ENTERPRISE_TIER;
	// Enterprise accounts are managed outside Stripe. Never route them into a
	// fresh checkout or the (nonexistent) billing portal.
	const isEnterpriseCustomer = currentTier === ENTERPRISE_TIER;
	const highlighted = emphasize ?? (tier.highlight && !isCurrentTier);
	const displayName = tier.display_name ?? tier.name ?? tierKey;
	const badgeLabel = tier.badge ?? "Suggested plan";
	const colorClass = TIER_COLORS[tierKey] ?? TIER_COLORS.FREE;
	const icon = TIER_ICONS[tierKey] ?? TIER_ICONS.FREE;

	const metrics = [
		{
			resource: "cloud_runtime_ms",
			label: "Cloud runtime",
			value:
				tier.max_runtime_ms === undefined
					? "Not specified"
					: formatQuota(tier.max_runtime_ms, "cloud_runtime_ms"),
			period: "/ month",
		},
		{
			resource: "hosted_ai_cost_micros",
			label: "Hosted AI allowance",
			value: formatQuota(
				tier.max_ai_cost_micros ?? tier.max_llm_cost * 10_000,
				"hosted_ai_cost_micros",
			),
			period: "/ month",
		},
		{
			resource: "storage_bytes",
			label: "Cloud storage",
			value: formatBytes(tier.max_total_size),
			period: "stored",
		},
	];
	const extraLimits = [
		{
			resource: "cloud_starts",
			label: "Cloud starts / month",
			value: formatQuota(tier.max_remote_executions, "cloud_starts"),
		},
		{
			resource: "hosted_ai_calls",
			label: "Hosted AI operations / month",
			value:
				tier.max_llm_calls === undefined
					? "Not specified"
					: formatQuota(tier.max_llm_calls, "hosted_ai_calls"),
		},
		{
			resource: "projects",
			label: "Private cloud projects",
			value: formatQuota(tier.max_non_visible_projects, "projects"),
		},
		{
			resource: "concurrent_cloud_executions",
			label: "Concurrent cloud executions",
			value:
				tier.max_concurrent_executions === undefined
					? "Not specified"
					: formatQuota(
							tier.max_concurrent_executions,
							"concurrent_cloud_executions",
						),
		},
		{
			resource: "hosted_model_access",
			label: "Hosted model access",
			value:
				tier.llm_tiers
					.map((name) => name.charAt(0) + name.slice(1).toLowerCase())
					.join(", ") || "None",
		},
	];
	const fallbackFeatures = deriveTierFeatures(tier);
	const additionalFeatures =
		tier.features?.filter((feature) => !fallbackFeatures.includes(feature)) ??
		[];
	const action = (
		<div>
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
						hasExistingSubscription
							? onManageBilling()
							: onUpgrade(
									tierKey,
									tier.price?.id,
									tier.price?.interval === "year" ? "year" : "month",
								)
					}
					disabled={isLoading || (!hasExistingSubscription && !tier.price)}
				>
					{isLoading ? (
						<>
							<Loader2 className="h-4 w-4 animate-spin" />
							Processing...
						</>
					) : hasExistingSubscription ? (
						"Change plan"
					) : !tier.price ? (
						"Currently unavailable"
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
	);
	const price = (
		<div className="flex flex-wrap items-baseline gap-1">
			{isEnterprise ? (
				<span
					className={cn(
						"font-bold tracking-tight",
						compact ? "text-2xl" : "text-3xl",
					)}
				>
					{t("custom", "Custom")}
				</span>
			) : tier.price ? (
				<>
					<span
						className={cn(
							"font-bold tracking-tight",
							compact ? "text-2xl" : "text-3xl",
						)}
					>
						{formatPrice(tier.price.amount, tier.price.currency)}
					</span>
					{tier.price.interval && (
						<span className="text-sm text-muted-foreground">
							/{tier.price.interval}
						</span>
					)}
				</>
			) : isPaid ? (
				<span className="text-sm text-muted-foreground">Price unavailable</span>
			) : (
				<span
					className={cn(
						"font-bold tracking-tight",
						compact ? "text-2xl" : "text-3xl",
					)}
				>
					{t("free", "Free")}
				</span>
			)}
		</div>
	);

	return (
		<article
			className={cn(
				"relative flex h-full min-w-0 flex-col rounded-xl border bg-card transition-shadow",
				compact ? "min-h-[250px] p-4" : "p-6",
				highlighted
					? "border-primary shadow-floating ring-1 ring-primary/30"
					: "border-border dark:border-white/15",
				isCurrentTier && "border-primary/40",
			)}
		>
			{highlighted && (
				<Badge className="absolute -top-2.5 left-1/2 -translate-x-1/2 whitespace-nowrap bg-primary text-primary-foreground shadow-md">
					{badgeLabel}
				</Badge>
			)}
			{isCurrentTier && !highlighted && (
				<Badge
					variant="outline"
					className="absolute -top-2.5 left-1/2 -translate-x-1/2 whitespace-nowrap bg-card"
				>
					{t("currentPlan", "Current plan")}
				</Badge>
			)}
			<div className="flex items-start justify-between gap-3">
				<div className="flex min-w-0 items-center gap-2.5">
					<div
						className={cn(
							"flex shrink-0 items-center justify-center rounded-lg",
							compact ? "h-8 w-8" : "h-10 w-10",
							colorClass,
						)}
					>
						{icon}
					</div>
					<h3 className="break-words text-lg font-semibold leading-tight">
						{displayName}
					</h3>
				</div>
				{compact && <div className="shrink-0 text-right">{price}</div>}
			</div>
			{tier.tagline && (
				<p
					className={cn(
						"mt-2 text-sm leading-snug text-muted-foreground",
						!compact && "min-h-10",
					)}
				>
					{tier.tagline}
				</p>
			)}
			{!compact && <div className="mt-4">{price}</div>}
			{compact && <div className="mt-3">{action}</div>}
			<dl
				className={cn(
					compact
						? "mt-4 grid grid-cols-3 gap-3"
						: "my-5 divide-y divide-border",
				)}
			>
				{metrics.map(({ resource, label, value, period }) => (
					<div
						key={resource}
						className={cn(
							compact
								? "min-w-0"
								: "flex items-center justify-between gap-3 py-3",
							focusResource === resource && "text-primary",
						)}
					>
						<dt
							className={cn(
								"text-xs leading-snug",
								focusResource !== resource && "text-muted-foreground",
							)}
						>
							{label}
						</dt>
						<dd className={cn(compact ? "mt-1" : "text-right")}>
							<span className="block text-base font-semibold tabular-nums">
								{isEnterprise ? "Custom" : value}
							</span>
							<span className="block text-[11px] text-muted-foreground">
								{isEnterprise ? "by agreement" : period}
							</span>
							{resource === "cloud_runtime_ms" &&
								!isEnterprise &&
								runEstimate?.runs != null && (
									<span className="mt-1 block text-xs text-muted-foreground">
										≈ {runEstimate.runs.toLocaleString()} runs
									</span>
								)}
						</dd>
					</div>
				))}
			</dl>
			{!compact && <div className="mt-auto">{action}</div>}
			{showDetails && (
				<details className="group mt-4 border-t pt-3 text-sm">
					<summary className="flex cursor-pointer list-none items-center justify-between gap-2 text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
						More plan limits
						<ChevronDown className="h-4 w-4 transition-transform group-open:rotate-180" />
					</summary>
					<dl className="mt-3 space-y-3">
						{extraLimits.map(({ resource, label, value }) => (
							<div
								key={resource}
								className={cn(
									"flex justify-between gap-3 text-xs",
									focusResource === resource && "font-semibold text-primary",
								)}
							>
								<dt>{label}</dt>
								<dd className="text-right">
									{isEnterprise ? "By agreement" : value}
								</dd>
							</div>
						))}
					</dl>
					{additionalFeatures.length > 0 && (
						<div className="mt-4 space-y-2 text-xs">
							<p className="font-medium">Also included</p>
							<ul className="list-disc space-y-1.5 pl-4 text-muted-foreground">
								{additionalFeatures.map((feature) => (
									<li key={feature}>{feature}</li>
								))}
							</ul>
						</div>
					)}
					<p className="mt-3 text-xs leading-relaxed text-muted-foreground">
						Runtime, cloud starts and hosted AI allowances renew monthly.
						Storage, projects and concurrent execution slots measure current
						use.
					</p>
				</details>
			)}
		</article>
	);
}
