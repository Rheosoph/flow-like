"use client";

import type { ReactNode } from "react";
import { monoNames } from "./obj-name";
import { cx } from "./tone";

const HEADLINE_NAME = "text-[0.9em]";

/** SPEC §4.24: the page's one conclusion sentence, directly under the page header. */
export function Headline({
	lead,
	rest,
	names,
	className,
}: Readonly<{
	/** "warehouse-pi needs you now." */
	lead: ReactNode;
	/** The supporting sentences, muted. */
	rest?: ReactNode;
	/** Device and service names in plain-string sentences, set in mono (`headlineCopy().names`). */
	names?: readonly string[];
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
				{monoNames(lead, names, HEADLINE_NAME)}
			</span>
			{rest ? (
				<span className="max-w-[72ch] text-[15px] leading-5.5 font-normal text-muted-foreground">
					{monoNames(rest, names, HEADLINE_NAME)}
				</span>
			) : null}
		</p>
	);
}
