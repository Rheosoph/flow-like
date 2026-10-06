"use client";

import { TONE_TEXT, cx } from "../../../settings/devices/primitives/tone";
import { RUN_STATUS_TONE, type RunStatus } from "../contracts";
import { RUN_STATUS_ICON, RUN_STATUS_SPINS } from "../status-look";

/** A run status as its icon in its tone; the word is always printed beside it (never colour alone). */
export function StatusIcon({
	status,
	className,
}: Readonly<{ status: RunStatus; className?: string }>) {
	const Icon = RUN_STATUS_ICON[status];
	return (
		<Icon
			aria-hidden
			className={cx(
				"shrink-0",
				TONE_TEXT[RUN_STATUS_TONE[status]],
				RUN_STATUS_SPINS[status] && "animate-spin motion-reduce:animate-none",
				className,
			)}
		/>
	);
}
