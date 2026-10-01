"use client";

import type { ReactNode } from "react";
import { cx } from "./tone";

/** SPEC §4.24: the page's one conclusion sentence, directly under the page header. */
export function Headline({
	lead,
	rest,
	className,
}: Readonly<{
	/** "warehouse-pi needs you now." */
	lead: ReactNode;
	/** The supporting sentences, muted. */
	rest?: ReactNode;
	className?: string;
}>) {
	return (
		<p
			data-headline=""
			className={cx(
				"flex min-w-0 flex-col gap-1 text-headline font-medium text-pretty",
				className,
			)}
		>
			<span className="max-w-[60ch] font-semibold tracking-[-0.01em] text-foreground">
				{lead}
			</span>
			{rest ? (
				<span className="max-w-[72ch] text-[15px] leading-5.5 font-normal text-muted-foreground">
					{rest}
				</span>
			) : null}
		</p>
	);
}
