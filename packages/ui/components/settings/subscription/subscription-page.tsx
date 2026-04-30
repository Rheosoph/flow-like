"use client";

import { Check, Crown, Loader2, Mail, Sparkles, Zap } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type {
	IPricingResponse,
	ITierInfo,
} from "../../../state/backend-state/user-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Separator } from "../../ui/separator";

interface TierCardProps {
	tierKey: string;
	tier: ITierInfo;
	isCurrentTier: boolean;
	currentTier: string;
	onUpgrade: (tier: string) => Promise<void>;
	onManageBilling: () => Promise<void>;
	isLoading: boolean;
}

const TIER_ORDER = ["FREE", "PREMIUM", "PRO", "ENTERPRISE"];
const TIER_COLORS: Record<string, string> = {
	FREE: "bg-muted",
	PREMIUM: "bg-gradient-to-br from-amber-500 to-orange-600",
	PRO: "bg-gradient-to-br from-violet-500 to-purple-600",
	ENTERPRISE: "bg-gradient-to-br from-blue-500 to-indigo-600",
};

const TIER_ICONS: Record<string, React.ReactNode> = {
	FREE: <Zap className="h-5 w-5" />,
	PREMIUM: <Sparkles className="h-5 w-5" />,
	PRO: <Crown className="h-5 w-5" />,
	ENTERPRISE: <Crown className="h-5 w-5" />,
};

const ENTERPRISE_TIER = "ENTERPRISE";
const ENTERPRISE_CONTACT_URL = "mailto:enterprise@flow-like.com";

function formatBytes(bytes: number): string {
	if (bytes === 0) return "0 B";
	const k = 1024;
	const sizes = ["B", "KB", "MB", "GB", "TB"];
	const i = Math.floor(Math.log(bytes) / Math.log(k));
	return `${Number.parseFloat((bytes / k ** i).toFixed(2))} ${sizes[i]}`;
}

function formatPrice(
	amount: number,
	currency: string,
	interval?: string,
): string {
	const formatter = new Intl.NumberFormat("en-US", {
		style: "currency",
		currency: currency.toUpperCase(),
	});
	const formatted = formatter.format(amount / 100);
	return interval ? `${formatted}/${interval}` : formatted;
}

function TierCard({
	tierKey,
	tier,
	isCurrentTier,
	currentTier,
	onUpgrade,
	onManageBilling,
	isLoading,
}: TierCardProps) {
	const features = useMemo(() => {
		const items: string[] = [];
		if (tier.max_non_visible_projects > 0) {
			items.push(`${tier.max_non_visible_projects} private projects`);
		}
		if (tier.max_remote_executions > 0) {
			items.push(`${tier.max_remote_executions} remote executions/month`);
		}
		if (tier.max_total_size > 0) {
			items.push(`${formatBytes(tier.max_total_size)} storage`);
		}
		if (tier.max_llm_cost > 0) {
			items.push(`$${(tier.max_llm_cost / 100).toFixed(2)} LLM credits/month`);
		}
		if (tier.llm_tiers.length > 0) {
			items.push(`Access to ${tier.llm_tiers.join(", ")} models`);
		}
		return items;
	}, [tier]);

	const isPaid = tierKey !== "FREE" && tier.product_id;
	const isEnterprise = tierKey === ENTERPRISE_TIER;
	const hasExistingSubscription = currentTier !== "FREE";
	const colorClass = TIER_COLORS[tierKey] || TIER_COLORS.FREE;
	const icon = TIER_ICONS[tierKey] || TIER_ICONS.FREE;
	const contactUrl = tier.contact_url ?? ENTERPRISE_CONTACT_URL;

	return (
		<Card
			className={`relative flex flex-col ${isCurrentTier ? "border-primary border-2" : ""}`}
		>
			{isCurrentTier && (
				<Badge
					className="absolute -top-2 left-1/2 -translate-x-1/2"
					variant="default"
				>
					Current Plan
				</Badge>
			)}
			<CardHeader>
				<div
					className={`w-12 h-12 rounded-lg flex items-center justify-center text-white mb-4 ${colorClass}`}
				>
					{icon}
				</div>
				<CardTitle className="text-xl">{tier.name || tierKey}</CardTitle>
				<CardDescription>
					{isEnterprise ? (
						<span className="text-2xl font-bold text-foreground">Custom</span>
					) : tier.price ? (
						<span className="text-2xl font-bold text-foreground">
							{formatPrice(
								tier.price.amount,
								tier.price.currency,
								tier.price.interval,
							)}
						</span>
					) : (
						<span className="text-2xl font-bold text-foreground">Free</span>
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex-1">
				<ul className="space-y-2">
					{features.map((feature) => (
						<li key={feature} className="flex items-start gap-2">
							<Check className="h-4 w-4 text-green-500 mt-0.5 shrink-0" />
							<span className="text-sm text-muted-foreground">{feature}</span>
						</li>
					))}
				</ul>
			</CardContent>
			<CardFooter>
				{isCurrentTier ? (
					<Button className="w-full" variant="outline" disabled>
						Current Plan
					</Button>
				) : isEnterprise ? (
					<Button className="w-full" asChild>
						<a href={contactUrl}>
							<Mail className="h-4 w-4" />
							Contact Enterprise
						</a>
					</Button>
				) : isPaid ? (
					<Button
						className="w-full"
						onClick={() =>
							hasExistingSubscription ? onManageBilling() : onUpgrade(tierKey)
						}
						disabled={isLoading}
					>
						{isLoading ? (
							<>
								<Loader2 className="mr-2 h-4 w-4 animate-spin" />
								Processing...
							</>
						) : hasExistingSubscription ? (
							"Change Plan"
						) : (
							`Upgrade to ${tier.name || tierKey}`
						)}
					</Button>
				) : (
					<Button className="w-full" variant="outline" disabled>
						Default Plan
					</Button>
				)}
			</CardFooter>
		</Card>
	);
}

interface SubscriptionPageProps {
	pricing: IPricingResponse;
	onUpgrade: (tier: string) => Promise<void>;
	onManageBilling: () => Promise<void>;
	isPremiumEnabled?: boolean;
}

export function SubscriptionPage({
	pricing,
	onUpgrade,
	onManageBilling,
	isPremiumEnabled = true,
}: SubscriptionPageProps) {
	const [loadingTier, setLoadingTier] = useState<string | null>(null);

	const handleUpgrade = useCallback(
		async (tier: string) => {
			setLoadingTier(tier);
			try {
				await onUpgrade(tier);
			} finally {
				setLoadingTier(null);
			}
		},
		[onUpgrade],
	);

	const sortedTiers = useMemo(() => {
		const entries = Object.entries(pricing.tiers);
		return entries.sort((a, b) => {
			const aIndex = TIER_ORDER.indexOf(a[0]);
			const bIndex = TIER_ORDER.indexOf(b[0]);
			if (aIndex === -1 && bIndex === -1) return 0;
			if (aIndex === -1) return 1;
			if (bIndex === -1) return -1;
			return aIndex - bIndex;
		});
	}, [pricing.tiers]);

	if (!isPremiumEnabled) {
		return (
			<div className="container max-w-4xl mx-auto p-6">
				<div className="text-center py-12">
					<Crown className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
					<h2 className="text-2xl font-bold mb-2">Premium Features Disabled</h2>
					<p className="text-muted-foreground">
						Premium subscription features are not available on this instance.
					</p>
				</div>
			</div>
		);
	}

	return (
		<div className="container max-w-6xl mx-auto p-6 space-y-8">
			<div className="text-center space-y-2">
				<h1 className="text-3xl font-bold tracking-tight">Choose Your Plan</h1>
				<p className="text-muted-foreground max-w-2xl mx-auto">
					Unlock more features and capabilities with our premium plans. All
					plans include access to the core FlowLike platform.
				</p>
			</div>

			<div className="grid gap-6 md:grid-cols-2 lg:grid-cols-4">
				{sortedTiers.map(([key, tier]) => (
					<TierCard
						key={key}
						tierKey={key}
						tier={tier}
						isCurrentTier={pricing.current_tier === key}
						currentTier={pricing.current_tier}
						onUpgrade={handleUpgrade}
						onManageBilling={onManageBilling}
						isLoading={loadingTier === key}
					/>
				))}
			</div>

			{pricing.current_tier !== "FREE" && (
				<>
					<Separator />
					<div className="flex flex-col items-center gap-4">
						<p className="text-sm text-muted-foreground">
							Need to update your payment method or cancel your subscription?
						</p>
						<Button variant="outline" onClick={onManageBilling}>
							Manage Billing
						</Button>
					</div>
				</>
			)}
		</div>
	);
}
