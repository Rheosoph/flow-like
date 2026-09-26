"use client";
import {
	type DeserializeMdOptions,
	MarkdownPlugin,
	remarkMdx,
	remarkMention,
} from "@platejs/markdown";
import { KEYS, TextApi, type Value } from "platejs";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { deserializeMdKeepingCode } from "./markdown-code-guard";
import { withoutUnsafeUrls } from "./safe-url-kit";

export const MarkdownKit = [
	MarkdownPlugin.configure({
		options: {
			disallowedNodes: [KEYS.suggestion],
			plainMarks: [KEYS.comment],
			remarkPlugins: [remarkMath, remarkGfm, remarkMdx, remarkMention],
		},
	}).extendApi(({ editor }) => ({
		/** Links and media with script or non-image `data:` URLs never leave the parser. */
		deserialize: (
			data: string,
			options?: Omit<DeserializeMdOptions, "editor">,
		): Value =>
			withoutUnsafeUrls(
				editor,
				deserializeMdKeepingCode(editor, data, options),
			).map((node) =>
				TextApi.isText(node)
					? { type: editor.getType(KEYS.p), children: [node] }
					: node,
			),
	})),
];
