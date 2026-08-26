"use client";

import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import {
	AppWindowIcon,
	ChevronDownIcon,
	ExternalLinkIcon,
	XIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { Button } from "../../index";
import type { InlineAppPage } from "../../state/global-chat/global-chat-store";
import {
	InlineAppPageSlot,
	getInlineAppPageTarget,
	subscribeToInlineAppPagePresentation,
} from "./inline-app-page-runtime";

interface InlineAppPageCardProps {
	page: InlineAppPage;
	onClose: (id: string) => void;
	/** Tighter height when rendered inside the docked overlay. */
	compact?: boolean;
}

/**
 * A visible slot for an app page owned by InlineAppPageRuntimeHost. Collapsing this card parks the
 * live page without unloading it; closing removes the store entry and tears the runtime down.
 */
export function InlineAppPageCard({
	page,
	onClose,
	compact = false,
}: InlineAppPageCardProps) {
	const { t } = useTranslation("chat");
	const router = useRouter();
	const [expanded, setExpanded] = useState(true);
	const contentId = `inline-app-page-content-${page.id}`;
	const titleId = `inline-app-page-title-${page.id}`;

	useEffect(() => {
		return subscribeToInlineAppPagePresentation((detail) => {
			if (detail?.appId !== page.appId || detail.eventId !== page.eventId)
				return;
			setExpanded(true);
		});
	}, [page.appId, page.eventId]);

	const openFullView = () => {
		const target = getInlineAppPageTarget(page.id) ?? {
			routePath: "/",
			eventId: page.eventId ?? null,
			queryParams: {},
		};
		const params = new URLSearchParams({ id: page.appId });
		if (target.eventId) params.set("eventId", target.eventId);
		else params.set("route", target.routePath);
		for (const [key, value] of Object.entries(target.queryParams)) {
			if (key !== "id" && key !== "route" && key !== "eventId") {
				params.set(key, value);
			}
		}
		onClose(page.id);
		router.push(`/use?${params.toString()}`);
	};

	return (
		<motion.div
			// Position only: a size-animating layout projection scales the card, and the live
			// page inside has no layout of its own to counter-scale with, so expanding the card
			// visibly squashed and stretched the rendered page for the length of the spring.
			layout="position"
			initial={{ opacity: 0, y: 8 }}
			animate={{ opacity: 1, y: 0 }}
			exit={{ opacity: 0, y: 8 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="mx-auto mb-2 w-[calc(100%_-_1.5rem)] max-w-6xl shrink-0 overflow-hidden rounded-2xl border border-border/80 bg-card shadow-sm"
		>
			<div className="flex h-11 items-center justify-between gap-2 px-2.5">
				<button
					type="button"
					className="flex min-w-0 flex-1 items-center gap-2 rounded-lg px-1.5 py-1 text-left outline-none hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					onClick={() => setExpanded((open) => !open)}
					aria-expanded={expanded}
					aria-controls={contentId}
				>
					<span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
						<AppWindowIcon className="size-3.5" />
					</span>
					<span id={titleId} className="truncate text-[13px] font-semibold">
						{page.name}
					</span>
					<span className="shrink-0 text-[11px] text-muted-foreground">
						{t("appPage", "App Page")}
					</span>
					<ChevronDownIcon
						className={`ml-auto size-4 shrink-0 text-muted-foreground transition-transform ${expanded ? "" : "-rotate-90"}`}
					/>
				</button>
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8 shrink-0 rounded-lg text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					aria-label={t("openInFullAppView", "Open in full app view")}
					title={t("openInFullAppView", "Open in full app view")}
					onClick={openFullView}
				>
					<ExternalLinkIcon className="size-3.5" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8 shrink-0 rounded-lg text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					aria-label={t("closeAppPage", "Close app page")}
					onClick={() => onClose(page.id)}
				>
					<XIcon className="size-3.5" />
				</Button>
			</div>

			{expanded && (
				<InlineAppPageSlot
					pageId={page.id}
					id={contentId}
					role="region"
					aria-labelledby={titleId}
					className={`${compact ? "h-95 max-h-[calc(50vh-3.5rem)]" : "h-120 max-h-[calc(60vh-4.5rem)]"} relative flex flex-col overflow-hidden border-t border-border/70 bg-background contain-[layout_paint]`}
				/>
			)}
		</motion.div>
	);
}
