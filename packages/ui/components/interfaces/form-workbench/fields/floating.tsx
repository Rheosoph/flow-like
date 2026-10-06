"use client";

import * as PopoverPrimitive from "@radix-ui/react-popover";
import type { ComponentProps } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { usePortalContainer } from "../../../ui/portal-container";

/*
 * The portal layer of the recent-values list and the calendar. Both sit outside the interface root, so
 * they are styled from props and tokens, never from `/fw` container variants (PLAN §7). No animation:
 * a popover that closes by an action must not keep the page locked while it fades (memory
 * nested-radix-modal-pointer-lock), and these are never modal.
 */

export const FloatingRoot = PopoverPrimitive.Root;
export const FloatingAnchor = PopoverPrimitive.Anchor;
export const FloatingTrigger = PopoverPrimitive.Trigger;

const LAYER =
	"z-50 rounded-lg border border-border-strong bg-popover text-popover-foreground shadow-floating outline-hidden";

export function FloatingContent({
	className,
	...props
}: ComponentProps<typeof PopoverPrimitive.Content>) {
	const container = usePortalContainer();
	return (
		<PopoverPrimitive.Portal container={container}>
			<PopoverPrimitive.Content
				side="bottom"
				align="start"
				sideOffset={4}
				collisionPadding={8}
				className={cx(LAYER, className)}
				{...props}
			/>
		</PopoverPrimitive.Portal>
	);
}
