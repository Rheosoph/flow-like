"use client";

import type { CSSProperties } from "react";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";

export type UserAvatarSize = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

const AVATAR_SIZE_CLASSES: Record<string, string> = {
	xs: "h-5 w-5",
	sm: "h-6 w-6",
	md: "h-8 w-8",
	lg: "h-10 w-10",
	xl: "h-14 w-14",
	"2xl": "h-16 w-16",
};

const AVATAR_FALLBACK_CLASSES: Record<string, string> = {
	xs: "text-[9px]",
	sm: "text-[10px]",
	md: "text-xs",
	lg: "text-sm",
	xl: "text-base",
	"2xl": "text-lg",
};

/** Width of each named size, so a pixel size borrows the nearest type scale. */
const NAMED_SIZE_PIXELS: ReadonlyArray<readonly [UserAvatarSize, number]> = [
	["xs", 20],
	["sm", 24],
	["md", 32],
	["lg", 40],
	["xl", 56],
	["2xl", 64],
];

function namedSize(size: string | number): string {
	if (typeof size === "string") return size;
	return NAMED_SIZE_PIXELS.find(([, pixels]) => size <= pixels)?.[0] ?? "2xl";
}

/** The pixel width a named size renders at; unknown names render as `md`. */
export function userAvatarPixels(size: string | number): number {
	if (typeof size === "number") return size;
	return NAMED_SIZE_PIXELS.find(([name]) => name === size)?.[1] ?? 32;
}

/**
 * The avatar every identity surface renders, so a user is the same size and
 * shape in a table cell, a hover card and an a2ui page. A number is a size in
 * pixels.
 */
export function UserAvatar({
	avatarUrl,
	initials,
	label,
	size = "md",
	className,
	style,
	title,
}: Readonly<{
	avatarUrl?: string | null;
	initials: string;
	label: string;
	size?: string | number;
	className?: string;
	style?: CSSProperties;
	title?: string;
}>) {
	const named = namedSize(size);
	return (
		<Avatar
			className={cn(
				AVATAR_SIZE_CLASSES[named] ?? AVATAR_SIZE_CLASSES.md,
				"shrink-0",
				className,
			)}
			style={
				typeof size === "number"
					? { width: size, height: size, ...style }
					: style
			}
			title={title}
		>
			<AvatarImage src={avatarUrl ?? ""} alt={label} />
			<AvatarFallback
				className={AVATAR_FALLBACK_CLASSES[named] ?? AVATAR_FALLBACK_CLASSES.md}
			>
				{initials}
			</AvatarFallback>
		</Avatar>
	);
}
