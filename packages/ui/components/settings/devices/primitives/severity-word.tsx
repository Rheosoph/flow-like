"use client";

import { useTranslation } from "@flow-like/locales";
import type { DevicesT } from "./area-context";
import { SEVERITY_ICON, SEVERITY_TONE, type SeverityKind } from "./icons";
import { TONE_TEXT, cx } from "./tone";

export function severityLabel(t: DevicesT, severity: SeverityKind): string {
	const labels = {
		critical: t("devices:enum.severity.critical", "Critical"),
		warning: t("devices:enum.severity.warning", "Warning"),
		notice: t("devices:enum.severity.notice", "Notice"),
		info: t("devices:enum.severity.info", "Info"),
	} satisfies Record<SeverityKind, string>;
	return labels[severity];
}

/** SPEC §4.3: icon + word in the severity's tone. `label` = the 11 px uppercase form. */
export function SeverityWord({
	severity,
	variant = "word",
	iconOnly = false,
	className,
}: Readonly<{
	severity: SeverityKind;
	variant?: "word" | "label";
	/** Icon with the word as its accessible name (dense rail glyphs). */
	iconOnly?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const Icon = SEVERITY_ICON[severity];
	const label = severityLabel(t, severity);
	return (
		<span
			data-severity={severity}
			role={iconOnly ? "img" : undefined}
			aria-label={iconOnly ? label : undefined}
			className={cx(
				"inline-flex items-center gap-1 font-medium whitespace-nowrap",
				TONE_TEXT[SEVERITY_TONE[severity]],
				className,
			)}
		>
			<Icon
				aria-hidden
				className={variant === "label" ? "size-3.5" : "size-4"}
			/>
			{iconOnly ? null : (
				<span
					className={
						variant === "label"
							? "text-label font-semibold uppercase tracking-[0.06em]"
							: undefined
					}
				>
					{label}
				</span>
			)}
		</span>
	);
}
