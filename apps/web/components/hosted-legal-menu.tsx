"use client";

import { useFlowLikeLegalLinks } from "@flow-like/flow-like-ui/components/legal/legal-links";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@flow-like/flow-like-ui/components/ui/popover";
import {
	type IAppLegalLinks,
	safeExternalHref,
} from "@flow-like/flow-like-ui/lib/legal-links";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { useTranslation } from "@flow-like/locales";
import { Scale } from "lucide-react";
import { useMemo } from "react";

interface MenuLink {
	key: string;
	href: string;
	label: string;
}

function LinkGroup({
	title,
	links,
}: Readonly<{ title: string; links: readonly MenuLink[] }>) {
	return (
		<div className="space-y-0.5">
			<p className="px-2 pb-0.5 pt-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
				{title}
			</p>
			{links.map((link) => (
				<a
					key={link.key}
					href={link.href}
					target="_blank"
					rel="noopener noreferrer"
					className="block rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
				>
					{link.label}
				</a>
			))}
		</div>
	);
}

/**
 * Legal links of a hosted page: the publisher's own, when set, then the host's.
 * Pages without a header get a small floating button that app CSS can move via
 * `[data-fl-hosted-legal]`.
 */
export function HostedLegalMenu({
	legal,
	floating,
}: Readonly<{ legal?: IAppLegalLinks; floating: boolean }>) {
	const { t } = useTranslation("common");
	const host = useFlowLikeLegalLinks();
	const publisher = useMemo(
		() =>
			[
				{
					key: "notice",
					href: safeExternalHref(legal?.legal_notice_url),
					label: t("legal.notice", "Legal notice"),
				},
				{
					key: "privacy",
					href: safeExternalHref(legal?.privacy_policy_url),
					label: t("legal.privacy", "Privacy policy"),
				},
			].filter((link): link is MenuLink => link.href !== undefined),
		[legal?.legal_notice_url, legal?.privacy_policy_url, t],
	);
	const hostLinks = useMemo(
		() => host.map((link) => ({ ...link, key: link.kind })),
		[host],
	);

	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					data-fl-hosted-legal=""
					aria-label={t("legal.title", "Legal information")}
					title={t("legal.title", "Legal information")}
					className={cn(
						"size-7 shrink-0 text-muted-foreground",
						floating &&
							"fixed bottom-[calc(0.75rem+env(safe-area-inset-bottom))] left-3 z-40 rounded-full border bg-background/80 opacity-60 shadow-sm backdrop-blur hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100",
					)}
				>
					<Scale className="size-3.5" />
				</Button>
			</PopoverTrigger>
			<PopoverContent
				side={floating ? "top" : "bottom"}
				align={floating ? "start" : "end"}
				className="w-56 space-y-1 p-1.5"
			>
				{publisher.length > 0 && (
					<LinkGroup title={t("legal.thisApp", "This app")} links={publisher} />
				)}
				<LinkGroup
					title={t("legal.hostedBy", "Hosted with Flow-Like")}
					links={hostLinks}
				/>
			</PopoverContent>
		</Popover>
	);
}
