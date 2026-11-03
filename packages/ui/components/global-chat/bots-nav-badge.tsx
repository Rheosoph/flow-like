"use client";

import { usePersonUnreadTotal } from "../../hooks/use-inbox";
import { ChatActivityBadge } from "./chat-activity";

/** The Bots item's badge: chat activity plus unread messages in your person chats. */
export function BotsNavBadge({ className }: Readonly<{ className?: string }>) {
	const people = usePersonUnreadTotal();
	return <ChatActivityBadge className={className} extra={people} />;
}
