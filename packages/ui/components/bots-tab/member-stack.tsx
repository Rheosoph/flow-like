"use client";

import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import { IdentityHoverCard } from "../identity/identity-hover-card";

/** Overlapping avatars for the people and bots of a conversation, with "+N" for the rest. */
export function MemberStack({
	principalIds,
	max = 3,
	size = 30,
	className,
	hoverCards = false,
}: Readonly<{
	principalIds: readonly string[];
	max?: number;
	size?: number;
	className?: string;
	/** Off in lists, where every row would open cards as the pointer passes. */
	hoverCards?: boolean;
}>) {
	const shown = principalIds.slice(0, max);
	const hidden = principalIds.length - shown.length;
	const overlap = Math.round(size / 3.5);
	return (
		<span className={cn("flex shrink-0 items-center", className)}>
			{shown.map((id, index) => {
				const avatar = (
					<IdentityAvatar
						subject={id}
						size={size}
						className="ring-2 ring-card"
					/>
				);
				return (
					<span
						key={id}
						className="relative flex"
						style={{
							zIndex: shown.length - index,
							marginLeft: index > 0 ? -overlap : 0,
						}}
					>
						{hoverCards ? (
							<IdentityHoverCard subject={id}>{avatar}</IdentityHoverCard>
						) : (
							avatar
						)}
					</span>
				);
			})}
			{hidden > 0 && (
				<span
					className="relative inline-flex shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold text-muted-foreground ring-2 ring-card"
					style={{ width: size, height: size, marginLeft: -overlap }}
				>
					+{hidden}
				</span>
			)}
		</span>
	);
}
