"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import { type ReactNode, useState } from "react";
import { DvButton } from "./dv-button";
import { cx } from "./tone";
import { useCopy } from "./use-copy";

const GROUPS_SHOWN = 4;

/** Hex fingerprints: separators dropped, upper-cased. Case-sensitive values (`keep`) are only split. */
function groupsOf4(value: string, keep: boolean): string[] {
	const text = keep ? value : value.replace(/[^0-9a-z]/gi, "").toUpperCase();
	return text.match(/.{1,4}/gu) ?? [value];
}

/** SPEC §4.19 / IA §6.3.6: short form, reveal the full value, Copy with a "Copied" swap. */
export function IdRef({
	id,
	label,
	copyLabel,
	group4 = false,
	short = 8,
	title,
	className,
}: Readonly<{
	id: string;
	/** The `.idref-k` term ("Device ID"); omitted in tables. */
	label?: ReactNode;
	/** Accessible name of the copy button ("Copy device ID"). */
	copyLabel?: string;
	/**
	 * Fingerprints: blocks of 4 for reading aloud. `"keep"` leaves the value as
	 * it is (base64url fingerprints are case-sensitive); `true` is for hex.
	 */
	group4?: boolean | "keep";
	/** Characters shown before the reveal (a service ID is a name: pass its full length). */
	short?: number;
	/** Hover text of the value; the full value by default. */
	title?: string;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const [revealed, setRevealed] = useState(false);
	const { copied, copy } = useCopy();
	const groups = group4 ? groupsOf4(id, group4 === "keep") : null;
	const full = groups ? groups.join(" ") : id;
	const isShort = groups ? groups.length <= GROUPS_SHOWN : id.length <= short;
	const shortText = groups
		? `${groups.slice(0, GROUPS_SHOWN).join(" ")} …`
		: id.slice(0, short);
	return (
		<span
			data-idref=""
			className={cx(
				"inline-flex max-w-full min-w-0 items-center gap-0.5 align-middle",
				className,
			)}
		>
			{label ? (
				<span className="mr-1 text-xs whitespace-nowrap text-muted-foreground">
					{label}
				</span>
			) : null}
			<button
				type="button"
				title={title ?? id}
				aria-expanded={isShort ? undefined : revealed}
				onClick={() => setRevealed((current) => !current)}
				className={cx(
					"min-w-0 cursor-pointer rounded-md border border-hairline bg-surface-sunken px-1.5 py-px text-left font-mono text-xs text-ink-2 hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					revealed
						? "wrap-normal break-normal whitespace-normal"
						: "truncate whitespace-nowrap",
				)}
			>
				{revealed || isShort ? full : shortText}
			</button>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={
					copied
						? t("common.idref.copied", "Copied")
						: (copyLabel ?? t("common.idref.copy", "Copy ID"))
				}
				onClick={() => {
					void copy(id);
				}}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}
