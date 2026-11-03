"use client";

import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";

/** Overlapping avatars for the people and bots of a conversation, with "+N" for the rest. */
export function MemberStack({
	principalIds,
	max = 3,
	size = 30,
	className,
}: Readonly<{
	principalIds: readonly string[];
	max?: number;
	size?: number;
	className?: string;
}>) {
	const shown = principalIds.slice(0, max);
	const hidden = principalIds.length - shown.length;
	const overlap = Math.round(size / 3.5);
	return (
		<span className={cn("flex shrink-0 items-center", className)}>
			{shown.map((id, index) => (
				<span
					key={id}
					className="relative flex"
					style={{
						zIndex: shown.length - index,
						marginLeft: index > 0 ? -overlap : 0,
					}}
				>
					<IdentityAvatar
						subject={id}
						size={size}
						className="ring-2 ring-card"
					/>
				</span>
			))}
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
