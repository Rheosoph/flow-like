"use client";

import type { ReactNode } from "react";
import { cx } from "../../../settings/devices/primitives/tone";

export interface ZeroFieldHeroProps {
	readonly name: string;
	readonly description: string;
	/** A card in a split box (the Run button sits in it); bare text on a narrow box, where Run is in the phone dock. */
	readonly carded: boolean;
	/** The Run control, inside the card. */
	readonly children?: ReactNode;
}

/** A form without fields before its first run: the name, the description and one button, centred. */
export function ZeroFieldHero({
	name,
	description,
	carded,
	children,
}: Readonly<ZeroFieldHeroProps>) {
	return (
		<div
			data-fw-hero=""
			className={cx(
				"flex min-h-0 flex-1 flex-col overflow-y-auto py-8",
				carded ? "px-8" : "px-4",
			)}
		>
			<div className="m-auto w-full max-w-[30rem] pb-14">
				<div
					className={cx(
						"flex flex-col items-center gap-2.5 text-center",
						carded &&
							"rounded-[10px] border border-border bg-card px-8 pt-7 pb-8",
					)}
				>
					<h1 className="text-balance font-semibold text-2xl leading-[30px] tracking-[-0.015em]">
						{name}
					</h1>
					{description ? (
						<p className="text-balance text-[15px] text-ink-2 leading-6">
							{description}
						</p>
					) : null}
					{children ? <div className="mt-4 w-full">{children}</div> : null}
				</div>
			</div>
		</div>
	);
}
