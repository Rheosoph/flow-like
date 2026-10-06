import type { ReactNode } from "react";
import { cx } from "../../../settings/devices/primitives/tone";

/** The 11 px uppercase label that heads a body section (SURFACE §3); the base layer's h3 size and tracking are reset. */
export function SectionLabel({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<h3
			className={cx(
				"m-0 scroll-m-0 font-semibold text-label text-muted-foreground uppercase leading-4 tracking-[0.06em]",
				className,
			)}
		>
			{children}
		</h3>
	);
}

/**
 * A section: its label row (with whatever sits beside the label) and its content, anchored for the links.
 * `rowHeight` is the label row's least height (20 px; the Inputs row keeps 28 px for its button).
 */
export function Section({
	id,
	label,
	aside,
	gap = "gap-2.5",
	rowHeight = "min-h-5",
	className,
	children,
}: Readonly<{
	id: string;
	label: ReactNode;
	aside?: ReactNode;
	gap?: string;
	rowHeight?: string;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<section data-sec={id} className={cx("flex flex-col", gap, className)}>
			<div className={cx("flex items-center gap-2", rowHeight)}>
				<SectionLabel>{label}</SectionLabel>
				{aside}
			</div>
			{children}
		</section>
	);
}
