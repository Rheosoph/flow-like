"use client";

import { useState } from "react";
import { seedGradient } from "../../lib/seed-gradient";
import { userInitials } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";

/**
 * A fixed-size room portrait that fits beside a message preview: the room's picture, or up to
 * two members over the room's gradient while it has none or the picture fails to load.
 */
export function RoomAvatar({
	roomId,
	title,
	principalIds,
	imageUrl,
	size = 36,
	className,
}: Readonly<{
	roomId: string;
	title: string;
	principalIds: readonly string[];
	imageUrl?: string | null;
	size?: number;
	className?: string;
}>) {
	const [failedUrl, setFailedUrl] = useState<string | null>(null);
	const picture = imageUrl && imageUrl !== failedUrl ? imageUrl : null;
	const shown = principalIds.slice(0, 2);
	return (
		<span
			aria-hidden
			className={cn("relative inline-flex shrink-0", className)}
			style={{ width: size, height: size }}
		>
			{picture ? (
				<img
					src={picture}
					alt=""
					draggable={false}
					onError={() => setFailedUrl(picture)}
					className="size-full rounded-xl object-cover"
				/>
			) : (
				<>
					<span
						className="absolute inset-0 rounded-xl"
						style={{ backgroundImage: seedGradient(roomId) }}
					/>
					{shown.length === 0 ? (
						<span className="relative flex size-full items-center justify-center rounded-xl bg-black/50 text-xs font-semibold text-white">
							{userInitials(title)}
						</span>
					) : (
						shown.map((id, index) => (
							<span
								key={id}
								className={cn(
									"absolute flex",
									shown.length === 1
										? "inset-0 items-center justify-center"
										: index === 0
											? "top-0 left-0 z-10"
											: "right-0 bottom-0",
								)}
							>
								<IdentityAvatar
									subject={id}
									size={Math.round(size * (shown.length === 1 ? 0.78 : 0.67))}
									className="ring-2 ring-card"
								/>
							</span>
						))
					)}
				</>
			)}
		</span>
	);
}
