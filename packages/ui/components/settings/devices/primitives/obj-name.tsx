"use client";

import type { ReactNode } from "react";
import { cx } from "./tone";

/** A device or service name inside running text (R15): mono, a touch smaller, never broken across lines. */
export function ObjName({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<span
			data-obj=""
			className={cx(
				"font-mono text-[0.92em] font-semibold whitespace-nowrap text-foreground",
				className,
			)}
		>
			{children}
		</span>
	);
}

const escapeRegExp = (value: string) =>
	value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Sets the known device and service names of a translated sentence in mono.
 * A name only matches as a whole (`edge-01` never inside `edge-011`); anything
 * but a plain string passes through unchanged.
 */
export function monoNames(
	text: ReactNode,
	names?: readonly (string | null | undefined)[],
	className?: string,
): ReactNode {
	if (typeof text !== "string" || !names?.length) return text;
	const list = [...new Set(names.filter((name): name is string => !!name))];
	if (!list.length) return text;
	list.sort((a, b) => b.length - a.length);
	const pattern = new RegExp(
		`(^|[^\\w-])(${list.map(escapeRegExp).join("|")})(?![\\w-])`,
		"g",
	);
	const parts: ReactNode[] = [];
	let last = 0;
	for (const match of text.matchAll(pattern)) {
		const [, before = "", name = ""] = match;
		const start = (match.index ?? 0) + before.length;
		if (start > last) parts.push(text.slice(last, start));
		parts.push(
			<ObjName key={start} className={className}>
				{name}
			</ObjName>,
		);
		last = start + name.length;
	}
	if (!parts.length) return text;
	if (last < text.length) parts.push(text.slice(last));
	return parts;
}
