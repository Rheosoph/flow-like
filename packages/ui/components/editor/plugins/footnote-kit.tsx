"use client";

import { toPlatePlugin, type PlateEditor } from "platejs/react";
import {
	FOOTNOTE_DEFINITION,
	FOOTNOTE_REFERENCE,
	nextFootnoteIdentifier,
} from "../../../lib/plate-footnotes";
import { FootnoteDefinition, FootnoteReference } from "../ui/footnote-node";
import {
	BaseFootnoteDefinitionPlugin,
	BaseFootnoteReferencePlugin,
} from "./footnote-base-kit";

export const FootnoteKit = [
	toPlatePlugin(BaseFootnoteReferencePlugin, {
		node: { component: FootnoteReference },
	}),
	toPlatePlugin(BaseFootnoteDefinitionPlugin, {
		node: { component: FootnoteDefinition },
	}),
];

export function insertFootnote(editor: PlateEditor): string {
	const identifier = nextFootnoteIdentifier(editor.children);
	editor.tf.withoutNormalizing(() => {
		if (!editor.selection) editor.tf.select(editor.api.end([])!);
		editor.tf.insertNodes({
			type: FOOTNOTE_REFERENCE,
			identifier,
			children: [{ text: "" }],
		});
		const path = [editor.children.length];
		editor.tf.insertNodes(
			{
				type: FOOTNOTE_DEFINITION,
				identifier,
				children: [{ type: "p", children: [{ text: "" }] }],
			},
			{ at: path },
		);
		editor.tf.select({ path: [...path, 0, 0], offset: 0 });
	});
	editor.tf.focus();
	return identifier;
}
