"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { ChevronRight } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { cn } from "../../../lib/utils";

interface SpoilerBlockProps {
	content: string;
	className?: string;
}

function parseSpoilerContent(raw: string): {
	label: string;
	body: string;
} {
	const separatorIndex = raw.indexOf("\n---\n");
	if (separatorIndex !== -1) {
		const label = raw.slice(0, separatorIndex).trim();
		const body = raw.slice(separatorIndex + 5).trim();
		return { label: label || "Spoiler", body };
	}
	return { label: i18next.t('spoiler', 'Spoiler'), body: raw.trim() };
}

export function SpoilerBlock({ content, className }: SpoilerBlockProps) {
	const { t } = useTranslation("common");
	const [isOpen, setIsOpen] = useState(false);
	const bodyRef = useRef<HTMLDivElement>(null);
	const { label, body } = parseSpoilerContent(content);

	const toggle = useCallback(() => {
		const el = bodyRef.current;
		if (!el) {
			setIsOpen((v) => !v);
			return;
		}

		if (isOpen) {
			el.style.maxHeight = `${el.scrollHeight}px`;
			requestAnimationFrame(() => {
				el.style.maxHeight = "0px";
			});
			setTimeout(() => setIsOpen(false), 300);
		} else {
			setIsOpen(true);
			requestAnimationFrame(() => {
				el.style.maxHeight = `${el.scrollHeight}px`;
				setTimeout(() => {
					if (bodyRef.current) bodyRef.current.style.maxHeight = "none";
				}, 300);
			});
		}
	}, [isOpen]);

	return (
		<div
			className={cn(
				"my-2 rounded-md border border-border/50 bg-muted/30 overflow-hidden",
				className,
			)}
		>
			<button
				type="button"
				className="flex w-full items-center gap-2 px-4 py-3 text-sm font-medium hover:bg-muted/50 transition-colors rounded-t-md"
				onClick={toggle}
				aria-expanded={isOpen}
			>
				<ChevronRight
					className={cn(
						"size-4 shrink-0 transition-transform duration-300 ease-out",
						isOpen && "rotate-90",
					)}
				/>
				<span>{label}</span>
			</button>
			<div
				ref={bodyRef}
				className="transition-[max-height] duration-300 ease-out overflow-hidden"
				style={{ maxHeight: isOpen ? "none" : "0px" }}
			>
				<div className="px-4 pb-4 pt-1 text-sm leading-relaxed border-t border-border/30">
					<pre className="whitespace-pre-wrap font-sans">{body}</pre>
				</div>
			</div>
		</div>
	);
}

export default SpoilerBlock;
