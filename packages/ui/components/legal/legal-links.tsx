"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import {
	FLOW_LIKE_LEGAL_URLS,
	type LegalLinkKind,
} from "../../lib/legal-links";
import { cn } from "../../lib/utils";

export interface LegalLink {
	kind: LegalLinkKind;
	href: string;
	label: string;
}

export function useFlowLikeLegalLinks(): readonly LegalLink[] {
	const { t } = useTranslation("common");
	return useMemo(
		() => [
			{
				kind: "notice",
				href: FLOW_LIKE_LEGAL_URLS.notice,
				label: t("legal.notice", "Legal notice"),
			},
			{
				kind: "privacy",
				href: FLOW_LIKE_LEGAL_URLS.privacy,
				label: t("legal.privacy", "Privacy policy"),
			},
			{
				kind: "eula",
				href: FLOW_LIKE_LEGAL_URLS.eula,
				label: t("legal.eula", "EULA"),
			},
		],
		[t],
	);
}

export function LegalLinkRow({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("common");
	const links = useFlowLikeLegalLinks();
	return (
		<nav
			aria-label={t("legal.title", "Legal information")}
			className={cn(
				"flex flex-wrap items-center justify-center gap-x-3 gap-y-1 text-xs text-muted-foreground",
				className,
			)}
		>
			{links.map((link) => (
				<a
					key={link.kind}
					href={link.href}
					target="_blank"
					rel="noopener noreferrer"
					className="underline-offset-2 transition-colors hover:text-foreground hover:underline"
				>
					{link.label}
				</a>
			))}
		</nav>
	);
}
