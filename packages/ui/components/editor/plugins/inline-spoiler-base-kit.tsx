"use client";
import { createSlatePlugin } from "platejs";

import { InlineSpoilerElementStatic } from "../ui/inline-spoiler-static";

export const INLINE_SPOILER_KEY = "inline_spoiler";

export const BaseInlineSpoilerPlugin = createSlatePlugin({
	key: INLINE_SPOILER_KEY,
	node: {
		isElement: true,
		isInline: true,
		isVoid: true,
	},
});

export const BaseInlineSpoilerKit = [
	BaseInlineSpoilerPlugin.withComponent(InlineSpoilerElementStatic),
];
