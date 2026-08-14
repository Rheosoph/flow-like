"use client";

import { useTranslation } from "@flow-like/locales";
import { AnimatePresence, motion } from "framer-motion";
import {
	AppWindowIcon,
	ChevronDownIcon,
	ExternalLinkIcon,
	XIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { Button, UsePageContent } from "../../index";
import {
	INLINE_PAGE_REVEAL_EVENT,
	inlineAppPageSnapshotAttribute,
} from "../../lib/app-page-snapshot";
import { EVENT_CONFIG } from "../../lib/event-config";
import type { InlineAppPage } from "../../state/global-chat/global-chat-store";

interface InlineAppPageCardProps {
	page: InlineAppPage;
	onClose: (id: string) => void;
	/** Tighter height when rendered inside the docked overlay. */
	compact?: boolean;
}

/**
 * An app's UI page embedded inline in the global FlowPilot view — artifact-like, but backed by a
 * real, already-built app. Mounts the same use surface as the /use route (embedded mode), and the
 * header lets the user jump to the full app view.
 */
export function InlineAppPageCard({
	page,
	onClose,
	compact = false,
}: InlineAppPageCardProps) {
	const { t } = useTranslation("chat");
	const router = useRouter();
	const [expanded, setExpanded] = useState(false);
	const [target, setTarget] = useState<{
		routePath: string;
		eventId: string | null;
	}>({ routePath: "/", eventId: page.eventId ?? null });

	useEffect(() => {
		const reveal = (event: Event) => {
			const detail = (
				event as CustomEvent<{ appId?: string; eventId?: string }>
			).detail;
			if (detail?.appId !== page.appId || detail.eventId !== page.eventId)
				return;
			setExpanded(true);
			setTarget({ routePath: "/", eventId: page.eventId ?? null });
		};
		window.addEventListener(INLINE_PAGE_REVEAL_EVENT, reveal);
		return () => window.removeEventListener(INLINE_PAGE_REVEAL_EVENT, reveal);
	}, [page.appId, page.eventId]);

	const fullViewRoute = `/use?id=${page.appId}${
		target.eventId ? t('eventideventid', '&eventId={{eventId}}', { eventId: target.eventId }) : ""
	}`;

	return (
		<motion.div
			{...inlineAppPageSnapshotAttribute(page.appId, page.eventId)}
			layout
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, y: 8, scale: 0.98 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="mx-auto mb-2 w-[calc(100%_-_1.5rem)] max-w-6xl rounded-xl border border-border dark:border-white/20 bg-muted shadow-[0_12px_32px_-8px_rgba(0,0,0,0.35)] dark:shadow-[0_16px_40px_-8px_rgba(0,0,0,0.85)] overflow-hidden shrink-0"
		>
			<div className="flex items-center justify-between gap-2 px-3 py-2 bg-primary/5">
				<button
					type="button"
					className="flex items-center gap-2 min-w-0 flex-1 text-left rounded-md outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					onClick={() => setExpanded((open) => !open)}
					aria-expanded={expanded}
				>
					<span className="flex items-center justify-center size-6 rounded-md bg-primary/15 text-primary shrink-0">
						<AppWindowIcon className="size-3.5" />
					</span>
					<span className="text-[13px] font-semibold truncate">
						{page.name}
					</span>
					<span className="ml-1 px-1.5 py-0.5 rounded-full bg-primary/10 text-primary text-[10px] font-semibold uppercase tracking-wide shrink-0">
						{t('appPage', 'App Page')}
					</span>
					<ChevronDownIcon
						className={`size-4 text-muted-foreground shrink-0 ml-auto transition-transform ${expanded ? "" : "-rotate-90"}`}
					/>
				</button>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7 rounded-full shrink-0 text-muted-foreground hover:text-foreground outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					aria-label={t('openInFullAppView', 'Open in full app view')}
					title={t('openInFullAppView', 'Open in full app view')}
					onClick={() => router.push(fullViewRoute)}
				>
					<ExternalLinkIcon className="size-3.5" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7 rounded-full shrink-0 text-muted-foreground hover:text-foreground outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					aria-label={t('closeAppPage', 'Close app page')}
					onClick={() => onClose(page.id)}
				>
					<XIcon className="size-3.5" />
				</Button>
			</div>

			<AnimatePresence initial={false}>
				{expanded && (
					<motion.div
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1 }}
						exit={{ height: 0, opacity: 0 }}
						transition={{ duration: 0.2 }}
					>
						{/* [contain:layout_paint] makes this box the containing block for the page's
						    position:fixed/absolute a2ui widgets and clips their painting — without it
						    fixed-position page content escapes the card and bleeds over the chat. */}
						<div className="p-2 pt-1">
							<div
								className={`${compact ? "h-95 max-h-[calc(50vh-3.5rem)]" : "h-120 max-h-[calc(60vh-4.5rem)]"} relative overflow-hidden flex flex-col rounded-md border border-black/15 dark:border-black/60 bg-background contain-[layout_paint]`}
							>
								<UsePageContent
									eventConfig={EVENT_CONFIG}
									notFound={
										<div className="flex flex-1 items-center justify-center text-sm text-muted-foreground p-6">
											{t('thisAppPageIsNoLongerAvailable', 'This app page is no longer available.')}
										</div>
									}
									appId={page.appId}
									routePath={target.routePath}
									eventId={target.eventId}
									embedded
									onNavigate={(next) =>
										setTarget((current) => ({
											routePath: next.routePath ?? current.routePath,
											eventId:
												next.eventId === undefined
													? current.eventId
													: next.eventId,
										}))
									}
								/>
							</div>
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</motion.div>
	);
}
