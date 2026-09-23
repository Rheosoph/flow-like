"use client";

import type { SlateElementProps } from "platejs/static";

import { ChevronRight } from "lucide-react";
import { SlateElement } from "platejs/static";
import { useState } from "react";
import { Button } from "../../ui";

export function ToggleElementStatic(props: SlateElementProps) {
	const [open, setOpen] = useState(false);

	return (
		<SlateElement {...props} className="pl-6">
			<Button
				size="icon"
				variant="ghost"
				className="absolute top-0 -left-0.5 size-6 cursor-pointer items-center justify-center rounded-md p-px text-muted-foreground transition-colors select-none hover:bg-accent [&_svg]:size-4"
				contentEditable={false}
			>
				<ChevronRight
					className={
						open
							? "rotate-90 transition-transform duration-75"
							: "rotate-0 transition-transform duration-75"
					}
				/>
			</Button>
			{props.children}
		</SlateElement>
	);
}
