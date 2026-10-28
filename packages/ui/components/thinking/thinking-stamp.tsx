"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../lib/utils";
import { ThinkingGlyph } from "./thinking-glyph";
import { thinkingLevelForEffort, thinkingStampLabel } from "./thinking-options";
import { useThinkingLabels } from "./thinking-picker";

/** The thinking level a message was sent with, small enough to sit beside a message label. */
export function ThinkingStamp({
	value,
	className,
}: Readonly<{ value: string; className?: string }>) {
	const { t } = useTranslation("chat");
	const labels = useThinkingLabels();
	const label = thinkingStampLabel(value, labels);
	const description = t("thinkingLevelLabel", "Thinking: {{level}}", {
		level: label,
	});

	return (
		<span
			title={description}
			className={cn(
				"inline-flex items-center gap-1 text-[10px] font-medium normal-case tracking-normal text-muted-foreground/70",
				className,
			)}
		>
			<ThinkingGlyph level={thinkingLevelForEffort(value)} size="xs" idle />
			<span aria-hidden="true">{label}</span>
			<span className="sr-only">{description}</span>
		</span>
	);
}
