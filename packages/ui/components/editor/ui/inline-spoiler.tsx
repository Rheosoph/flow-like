"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { cn } from "../../../lib/utils";

interface InlineSpoilerProps {
	text: string;
	className?: string;
}

export function InlineSpoiler({ text, className }: InlineSpoilerProps) {
	const { t } = useTranslation("common");
	const [revealed, setRevealed] = useState(false);

	return (
		<span
			role="button"
			tabIndex={0}
			className={cn(
				"relative inline-block rounded px-1.5 py-0.5 transition-all duration-500 ease-out",
				revealed
					? "bg-muted/50 text-foreground cursor-default"
					: "cursor-pointer select-none",
				className,
			)}
			style={
				revealed
					? undefined
					: {
							color: "transparent",
							textShadow: "0 0 12px currentColor",
							background:
								"linear-gradient(135deg, hsl(var(--muted)/0.6), hsl(var(--muted)/0.9))",
							WebkitBackgroundClip: "padding-box",
						}
			}
			onClick={() => !revealed && setRevealed(true)}
			onKeyDown={(e) => {
				if (!revealed && (e.key === "Enter" || e.key === " ")) {
					e.preventDefault();
					setRevealed(true);
				}
			}}
			aria-label={
				revealed
					? text
					: t("hiddenContentClickToReveal", "Hidden content. Click to reveal.")
			}
		>
			{text}
			{!revealed && (
				<span
					className="absolute inset-0 rounded backdrop-blur-md bg-foreground/10 hover:bg-foreground/15 transition-colors duration-200"
					aria-hidden
				/>
			)}
		</span>
	);
}

export default InlineSpoiler;
