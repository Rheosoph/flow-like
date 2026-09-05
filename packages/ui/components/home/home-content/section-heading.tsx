"use client";

import { ArrowRight } from "lucide-react";
import Link from "next/link";
import { type HomeContentProps, safeHomeHref, textConfig } from "./config";

export function HomeSectionHeading({ widget }: HomeContentProps) {
	const href = safeHomeHref(textConfig(widget.config, "href"));
	const label = textConfig(widget.config, "linkLabel");
	return (
		<header
			data-home-section-heading
			className="flex min-w-0 flex-wrap items-end justify-between gap-x-5 gap-y-3 px-1 pt-5 pb-1"
		>
			<div className="min-w-0">
				<h2 className="text-xl font-semibold leading-7 tracking-tight">
					{widget.title || "Your section"}
				</h2>
				{widget.description && (
					<p className="mt-1 max-w-2xl text-xs leading-relaxed text-muted-foreground">
						{widget.description}
					</p>
				)}
			</div>
			{href && label && (
				<Link
					href={href}
					target={href.startsWith("/") ? undefined : "_blank"}
					rel={href.startsWith("/") ? undefined : "noopener noreferrer"}
					className="inline-flex shrink-0 items-center gap-2 rounded-lg border border-border/60 bg-card/50 px-3 py-2 text-xs font-medium text-muted-foreground transition-colors hover:border-primary/30 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{label}
					<ArrowRight className="size-3.5" aria-hidden="true" />
				</Link>
			)}
		</header>
	);
}
