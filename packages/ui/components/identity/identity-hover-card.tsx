"use client";

import type { ReactNode } from "react";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../ui/hover-card";
import { IdentityProfileContent } from "../ui/user-identity";

/**
 * Opens the identity card over anything that stands for a sub: a person's
 * profile, or a bot together with the person it acts for. Until the directory
 * answers there is nothing to show, so the children render bare.
 */
export function IdentityHoverCard({
	subject,
	children,
	className,
	align = "start",
	side,
}: Readonly<{
	subject: string;
	children: ReactNode;
	className?: string;
	align?: "start" | "center" | "end";
	side?: "top" | "right" | "bottom" | "left";
}>) {
	const identity = useUserIdentity(subject);
	const trigger = (
		<span className={cn("inline-flex min-w-0", className)}>{children}</span>
	);
	if (!identity.isResolved) return trigger;

	return (
		<HoverCard openDelay={300} closeDelay={120}>
			<HoverCardTrigger asChild>{trigger}</HoverCardTrigger>
			<HoverCardContent
				align={align}
				side={side}
				className="w-80 max-w-[calc(100vw-2rem)] overflow-hidden p-0"
			>
				<IdentityProfileContent identity={identity} />
			</HoverCardContent>
		</HoverCard>
	);
}
