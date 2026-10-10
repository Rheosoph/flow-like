"use client";

import { createSlatePlugin } from "platejs";
import {
	FOOTNOTE_DEFINITION,
	FOOTNOTE_REFERENCE,
} from "../../../lib/plate-footnotes";
import {
	FootnoteDefinitionStatic,
	FootnoteReferenceStatic,
} from "../ui/footnote-node-static";

export const BaseFootnoteReferencePlugin = createSlatePlugin({
	key: FOOTNOTE_REFERENCE,
	node: { isElement: true, isInline: true, isVoid: true },
});
export const BaseFootnoteDefinitionPlugin = createSlatePlugin({
	key: FOOTNOTE_DEFINITION,
	node: { isElement: true },
});
export const BaseFootnoteKit = [
	BaseFootnoteReferencePlugin.withComponent(FootnoteReferenceStatic),
	BaseFootnoteDefinitionPlugin.withComponent(FootnoteDefinitionStatic),
];
