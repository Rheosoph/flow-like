"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleCheck, OctagonX } from "lucide-react";
import type { ReactNode } from "react";
import { CONSEQ_ICON, type ConseqKind } from "./icons";
import { cx } from "./tone";

export interface UndoRow {
	/** `true` → "Yes." · `false` → "No, this is permanent." (tinted) · `null` → the text alone. */
	reversible: boolean | null;
	text?: ReactNode;
}

/** R8: `what`, `who`, `when` and `undo` are required, so no confirm can omit them. */
export interface ConsequenceRows {
	what: ReactNode;
	who: ReactNode;
	when: ReactNode;
	undo: UndoRow;
	stays?: ReactNode;
	first?: ReactNode;
}

export const CONSEQ_ORDER: readonly ConseqKind[] = [
	"what",
	"who",
	"stays",
	"when",
	"undo",
	"first",
];

function UndoValue({ undo }: Readonly<{ undo: UndoRow }>) {
	const { t } = useTranslation("devices");
	if (undo.reversible === null) return <>{undo.text}</>;
	const Icon = undo.reversible ? CircleCheck : OctagonX;
	return (
		<>
			<span
				className={cx(
					"inline-flex items-baseline gap-1 font-semibold",
					undo.reversible ? "text-good" : "text-critical",
				)}
			>
				<Icon aria-hidden className="size-3.5 shrink-0 translate-y-0.5" />
				{undo.reversible
					? t("common.conseq.undoYes", "Yes.")
					: t("common.conseq.undoNo", "No, this is permanent.")}
			</span>
			{undo.text ? <> {undo.text}</> : null}
		</>
	);
}

/** SPEC §4.6: fixed row order what · who · stays · when · undo · first. */
export function ConsequencePreview({
	rows,
	compact = false,
	whoLabel = "notices",
	labels,
	className,
}: Readonly<{
	rows: ConsequenceRows;
	compact?: boolean;
	/** "Who notices" or, for access changes, "Who loses access". */
	whoLabel?: "notices" | "loses";
	/** Row label overrides (e.g. the revoke flow's "Changes now"). */
	labels?: Partial<Record<ConseqKind, ReactNode>>;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const defaults: Record<ConseqKind, string> = {
		what: t("common.conseq.what", "What happens"),
		who:
			whoLabel === "loses"
				? t("common.conseq.whoLoses", "Who loses access")
				: t("common.conseq.who", "Who notices"),
		stays: t("common.conseq.stays", "What stays"),
		when: t("common.conseq.when", "When"),
		undo: t("common.conseq.undo", "Can you undo it?"),
		first: t("common.conseq.first", "Do this first"),
	};
	const values: Record<ConseqKind, ReactNode> = {
		what: rows.what,
		who: rows.who,
		stays: rows.stays,
		when: rows.when,
		undo: <UndoValue undo={rows.undo} />,
		first: rows.first,
	};
	const shown = CONSEQ_ORDER.filter(
		(kind) =>
			kind === "undo" || (values[kind] !== undefined && values[kind] !== null),
	);
	const cell = compact ? "px-2.5 py-1.5 text-[12.5px]" : "px-3 py-[9px]";

	return (
		<div className={cx("@container/cq min-w-0", className)}>
			<dl
				data-conseq=""
				className="grid grid-cols-[148px_minmax(0,1fr)] overflow-hidden rounded-lg border border-border bg-card text-ui @max-[560px]/cq:grid-cols-1"
			>
				{shown.map((kind, index) => {
					const Icon = CONSEQ_ICON[kind];
					const permanent = kind === "undo" && rows.undo.reversible === false;
					const divider = index > 0 && "border-t border-hairline";
					return (
						<div
							key={kind}
							data-kind={kind}
							data-reversible={
								kind === "undo" && rows.undo.reversible !== null
									? String(rows.undo.reversible)
									: undefined
							}
							className="contents"
						>
							<dt
								className={cx(
									"flex items-start gap-1.5 font-medium text-ink-2",
									permanent ? "bg-critical-bg" : "bg-surface-sunken",
									cell,
									divider,
								)}
							>
								<Icon
									aria-hidden
									className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
								/>
								{labels?.[kind] ?? defaults[kind]}
							</dt>
							<dd
								className={cx(
									"m-0 min-w-0 wrap-anywhere",
									permanent && "bg-critical-bg",
									cell,
									divider,
									"@max-[560px]/cq:border-t-0",
								)}
							>
								{values[kind]}
							</dd>
						</div>
					);
				})}
			</dl>
		</div>
	);
}
