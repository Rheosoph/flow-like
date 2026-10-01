"use client";

import type { ReactNode } from "react";
import { cx } from "./tone";

/** SPEC §4.17: two-column facts list; one column below 520 px of container width. */
export function KeyValueList({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<div className="@container/kv min-w-0">
			<dl
				className={cx(
					"grid grid-cols-[minmax(128px,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-ui @max-[520px]/kv:grid-cols-1 @max-[520px]/kv:gap-y-0.5",
					className,
				)}
			>
				{children}
			</dl>
		</div>
	);
}

export function KvRow({
	label,
	provenance,
	className,
	children,
}: Readonly<{
	label: ReactNode;
	/** Per-fact source, shown when it differs from the block's stamp ("live read 11 s ago"). */
	provenance?: ReactNode;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<div className="contents">
			<dt className="text-muted-foreground @max-[520px]/kv:mt-1.5">{label}</dt>
			<dd className={cx("m-0 min-w-0 wrap-anywhere", className)}>
				{children}
				{provenance ? (
					<span
						data-provenance=""
						className="ml-1.5 text-xs text-muted-foreground before:content-['('] after:content-[')']"
					>
						{provenance}
					</span>
				) : null}
			</dd>
		</div>
	);
}

/** A full-width sub-heading inside a list. */
export function KvGroup({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<div className="col-span-full pt-2 text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
			{children}
		</div>
	);
}
