"use client";

import { seedGradient } from "../../lib/seed-gradient";
import { userInitials } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";

/** A fixed-size room portrait that fits beside a message preview. */
export function RoomAvatar({
	roomId,
	title,
	principalIds,
	className,
}: Readonly<{
	roomId: string;
	title: string;
	principalIds: readonly string[];
	className?: string;
}>) {
	const shown = principalIds.slice(0, 2);
	return (
		<span
			aria-hidden
			className={cn("relative inline-flex size-9 shrink-0", className)}
		>
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
							size={shown.length === 1 ? 28 : 24}
							className="ring-2 ring-card"
						/>
					</span>
				))
			)}
		</span>
	);
}
