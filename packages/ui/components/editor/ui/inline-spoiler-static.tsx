"use client";

import type { SlateElementProps } from "platejs/static";
import { SlateElement } from "platejs/static";
import { InlineSpoiler } from "./inline-spoiler";

export interface TInlineSpoilerElement {
	type: "inline_spoiler";
	spoilerText: string;
	children: [{ text: "" }];
	[key: string]: unknown;
}

export function InlineSpoilerElementStatic(
	props: SlateElementProps<TInlineSpoilerElement>,
) {
	const { spoilerText } = props.element;

	return (
		<SlateElement {...props} as="span">
			<InlineSpoiler text={spoilerText || "???"} />
			<span className="hidden">{props.children}</span>
		</SlateElement>
	);
}
