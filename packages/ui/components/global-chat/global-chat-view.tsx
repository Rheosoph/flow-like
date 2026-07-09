"use client";

import { useEffect } from "react";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { GlobalChatBody } from "./global-chat-body";

/**
 * Full-page global chat surface for the /chat route. Closes the docked overlay while the page is
 * mounted so the conversation is only shown once, then hands rendering to the shared body.
 */
export function GlobalChatView() {
	const closeOverlay = useGlobalChatStore((s) => s.closeOverlay);
	useEffect(() => {
		closeOverlay();
	}, [closeOverlay]);

	return <GlobalChatBody variant="page" />;
}
