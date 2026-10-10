"use client";

import { cn } from "../../../lib/utils";

import { Toolbar } from "./toolbar";

export function FixedToolbar(props: React.ComponentProps<typeof Toolbar>) {
	return (
		<Toolbar
			aria-label="Document formatting"
			{...props}
			className={cn(
				"sticky top-0 left-0 z-50 @container/editor-toolbar w-full min-w-0 rounded-t-lg border-b border-b-border bg-background/95 backdrop-blur-xs supports-backdrop-blur:bg-background/60",
				props.className,
			)}
		/>
	);
}
