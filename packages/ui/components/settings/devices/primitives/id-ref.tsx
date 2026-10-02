"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import { type ReactNode, useState } from "react";
import { DvButton } from "./dv-button";
import { cx } from "./tone";
import { useCopy } from "./use-copy";

function groupsOf4(value: string): string {
	return (
		value
			.replace(/[^0-9a-z]/gi, "")
			.toUpperCase()
			.match(/.{1,4}/g)
			?.join(" ") ?? value
	);
}

/** SPEC §4.19 / IA §6.3.6: short form, reveal the full value, Copy with a "Copied" swap. */
export function IdRef({
	id,
	label,
	copyLabel,
	group4 = false,
	className,
}: Readonly<{
	id: string;
	/** The `.idref-k` term ("Device ID"); omitted in tables. */
	label?: ReactNode;
	/** Accessible name of the copy button ("Copy device ID"). */
	copyLabel?: string;
	/** Fingerprints: blocks of 4 for reading aloud. */
	group4?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const [revealed, setRevealed] = useState(false);
	const { copied, copy } = useCopy();
	const full = group4 ? groupsOf4(id) : id;
	const short = group4
		? `${full.split(" ").slice(0, 4).join(" ")} …`
		: id.slice(0, 8);
	const isShort = !group4 && id.length <= 8;
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
				title={id}
				aria-expanded={isShort ? undefined : revealed}
				onClick={() => setRevealed((current) => !current)}
				className={cx(
					"min-w-0 cursor-pointer rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px text-left font-mono text-xs text-ink-2 hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					revealed
						? "wrap-normal break-normal whitespace-normal"
						: "truncate whitespace-nowrap",
				)}
			>
				{revealed || isShort ? full : short}
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
