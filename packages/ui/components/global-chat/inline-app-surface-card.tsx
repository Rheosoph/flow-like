"use client";

import { AnimatePresence, motion } from "framer-motion";
import { ChevronDownIcon, LayoutGridIcon, XIcon } from "lucide-react";
import { useState } from "react";
import type { InlineAppSurface } from "../../state/global-chat/global-chat-store";
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { Button } from "../ui/button";

interface InlineAppSurfaceCardProps {
	surface: InlineAppSurface;
	onClose: (id: string) => void;
	/** Tighter height when rendered inside the docked overlay. */
	compact?: boolean;
}

/**
 * UI that an app pushed while FlowPilot talked to its chat (`call_app_chat`), shown inline in the
 * global chat. The run already finished, so this is a display-only snapshot of the surfaces the app
 * rendered — the assistant never sees this tree, it only relayed the app's text response.
 */
export function InlineAppSurfaceCard({
	surface,
	onClose,
	compact = false,
}: InlineAppSurfaceCardProps) {
	const [expanded, setExpanded] = useState(true);

	return (
		<motion.div
			layout
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, y: 8, scale: 0.98 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="mx-3 mb-2 rounded-xl border border-border dark:border-white/20 bg-muted shadow-[0_12px_32px_-8px_rgba(0,0,0,0.35)] dark:shadow-[0_16px_40px_-8px_rgba(0,0,0,0.85)] overflow-hidden shrink-0"
		>
			<div className="flex items-center justify-between gap-2 px-3 py-2 bg-primary/5">
				<button
					type="button"
					className="flex items-center gap-2 min-w-0 flex-1 text-left rounded-md outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					onClick={() => setExpanded((open) => !open)}
					aria-expanded={expanded}
				>
					<span className="flex items-center justify-center size-6 rounded-md bg-primary/15 text-primary shrink-0">
						<LayoutGridIcon className="size-3.5" />
					</span>
					<span className="text-[13px] font-semibold truncate">
						{surface.name}
					</span>
					<span className="ml-1 px-1.5 py-0.5 rounded-full bg-primary/10 text-primary text-[10px] font-semibold uppercase tracking-wide shrink-0">
						App UI
					</span>
					<ChevronDownIcon
						className={`size-4 text-muted-foreground shrink-0 ml-auto transition-transform ${expanded ? "" : "-rotate-90"}`}
					/>
				</button>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7 rounded-full shrink-0 text-muted-foreground hover:text-foreground outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					aria-label="Dismiss app UI"
					onClick={() => onClose(surface.id)}
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
						<div className="p-2 pt-1">
							<div
								className={`${compact ? "max-h-[50vh]" : "max-h-[60vh]"} overflow-y-auto rounded-md border border-black/15 dark:border-black/60 bg-background contain-[layout_paint]`}
							>
								{surface.surfaces.map((item) => (
									<A2UIRenderer
										key={item.id}
										surface={item}
										appId={surface.appId}
										isPreviewMode
										className="w-full"
									/>
								))}
							</div>
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</motion.div>
	);
}
