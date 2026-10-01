"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import { cx } from "./tone";

export type DiffKind = "added" | "changed" | "removed";

export interface DiffRow {
	kind: DiffKind;
	label: string;
	before?: ReactNode;
	after?: ReactNode;
}

const SIGN: Record<DiffKind, { glyph: string; cls: string }> = {
	added: { glyph: "+", cls: "border-good-line bg-good-bg text-good" },
	changed: { glyph: "~", cls: "border-info-line bg-info-bg text-info" },
	removed: {
		glyph: "−",
		cls: "border-warning-line bg-warning-bg text-warning",
	},
};

/**
 * Added / changed / removed rows with old → new values: the settings sheet,
 * deploy Review and change-permissions all render this one diff.
 */
export function DiffRows({
	rows,
	label,
	emptyText,
	className,
}: Readonly<{
	rows: readonly DiffRow[];
	/** Accessible name of the list ("Changes to settings v12"). */
	label?: string;
	emptyText?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const kindWord: Record<DiffKind, string> = {
		added: t("view.diff.added", "Added"),
		changed: t("view.diff.changed", "Changed"),
		removed: t("view.diff.removed", "Removed"),
	};
	if (!rows.length)
		return (
			<p className={cx("px-4 py-2 text-ui text-muted-foreground", className)}>
				{emptyText ?? t("view.diff.none", "No changes.")}
			</p>
		);
	return (
		<div className={cx("@container/diff min-w-0", className)}>
			<ul
				aria-label={label ?? t("view.diff.label", "Changes")}
				className="flex flex-col"
			>
				{rows.map((row, index) => {
					const sign = SIGN[row.kind];
					return (
						<li
							key={`${row.label}-${index}`}
							data-k={row.kind}
							className="grid grid-cols-[20px_minmax(110px,170px)_minmax(0,1fr)] items-start gap-x-3 gap-y-0.5 border-t border-hairline px-4 py-2 text-ui first:border-t-0 @max-[560px]/diff:grid-cols-[20px_minmax(0,1fr)] @max-[560px]/diff:px-3"
						>
							<span
								aria-hidden
								className={cx(
									"inline-flex size-4.5 items-center justify-center rounded-sm border font-mono text-xs leading-none font-semibold",
									sign.cls,
								)}
							>
								{sign.glyph}
							</span>
							<span className="text-muted-foreground">
								<span className="sr-only">
									{t("view.diff.kindSr", "{{kind}}: ", {
										kind: kindWord[row.kind],
									})}
								</span>
								{row.label}
							</span>
							<span className="min-w-0 wrap-break-word @max-[560px]/diff:col-start-2">
								{row.kind === "added" ? (
									row.after
								) : row.kind === "removed" ? (
									<s className="text-muted-foreground decoration-border-strong">
										{row.before}
									</s>
								) : (
									<>
										<s className="text-muted-foreground decoration-border-strong">
											{row.before}
										</s>
										<span aria-hidden className="px-1 text-muted-foreground">
											→
										</span>
										<span className="sr-only">
											{t("view.diff.toSr", " changed to ")}
										</span>
										{row.after}
									</>
								)}
							</span>
						</li>
					);
				})}
			</ul>
		</div>
	);
}
