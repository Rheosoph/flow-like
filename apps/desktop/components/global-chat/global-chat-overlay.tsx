"use client";

import { Button } from "@flow-like/flow-like-ui";
import { AnimatePresence, motion } from "framer-motion";
import { Maximize2Icon, SparklesIcon, XIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { createPortal } from "react-dom";
import { useGlobalChatStore } from "../../lib/global-chat-store";
import { GlobalChatBody } from "./global-chat-body";

/**
 * Docked bottom-right overlay for the global FlowPilot assistant. When the agent navigates the user
 * to a different view (via the navigate_view tool), the /chat conversation morphs into this floating
 * panel so the user keeps chatting while seeing the destination. Rendered into document.body so it
 * escapes sidebar/layout transforms, mirroring the RPA recording dock.
 */
export function GlobalChatOverlay() {
	const router = useRouter();
	const mode = useGlobalChatStore((s) => s.mode);
	const closeOverlay = useGlobalChatStore((s) => s.closeOverlay);

	const content = (
		<AnimatePresence>
			{mode === "overlay" && (
				<motion.div
					initial={{ opacity: 0, scale: 0.9, y: 24 }}
					animate={{ opacity: 1, scale: 1, y: 0 }}
					exit={{ opacity: 0, scale: 0.9, y: 24 }}
					transition={{ type: "spring", stiffness: 380, damping: 30 }}
					className="fixed bottom-6 right-6 z-[9999] pointer-events-none"
				>
					<div className="pointer-events-auto flex flex-col w-[min(440px,calc(100vw-3rem))] h-[640px] max-h-[calc(100vh-3rem)] rounded-2xl border border-border/60 bg-background/95 backdrop-blur-xl shadow-2xl overflow-hidden">
						<div className="flex items-center justify-between px-3 py-2 border-b border-border/50 shrink-0">
							<div className="flex items-center gap-1.5 text-sm font-medium">
								<SparklesIcon className="size-4 text-primary" />
								FlowPilot
							</div>
							<div className="flex items-center gap-1">
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7 rounded-full"
									aria-label="Open full chat"
									onClick={() => {
										router.push("/chat");
										closeOverlay();
									}}
								>
									<Maximize2Icon className="size-3.5" />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7 rounded-full"
									aria-label="Close chat"
									onClick={closeOverlay}
								>
									<XIcon className="size-4" />
								</Button>
							</div>
						</div>
						<div className="flex-1 min-h-0">
							<GlobalChatBody variant="overlay" />
						</div>
					</div>
				</motion.div>
			)}
		</AnimatePresence>
	);

	if (typeof document === "undefined") return content;
	return createPortal(content, document.body);
}
