"use client";

import { PlateElement, type PlateElementProps } from "platejs/react";

export function BlockquoteElement(props: PlateElementProps) {
	return (
		<PlateElement
			as="blockquote"
			className="my-1 border-l-2 pl-6 italic [&>.slate-p:first-child]:pt-0 [&>.slate-p:last-child]:pb-0"
			{...props}
		/>
	);
}
