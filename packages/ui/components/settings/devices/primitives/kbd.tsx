import type { ComponentProps } from "react";
import { cx } from "./tone";

/** Keyboard key hint (palette, rail filter "/"). */
export function Kbd({ className, ...props }: ComponentProps<"kbd">) {
	return (
		<kbd
			className={cx(
				"inline-flex h-4.5 items-center rounded-sm border border-border bg-surface-sunken px-1 font-mono text-label font-medium text-muted-foreground",
				className,
			)}
			{...props}
		/>
	);
}
