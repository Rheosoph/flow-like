"use client";

import { useTranslation } from "@flow-like/locales";
import * as React from "react";

import type { DropdownMenuProps } from "@radix-ui/react-dropdown-menu";

import { MarkdownPlugin } from "@platejs/markdown";
import { ArrowUpToLineIcon } from "lucide-react";
import { type SlateEditor, parseHtmlDocument } from "platejs";
import { useEditorRef } from "platejs/react";
import { useFilePicker } from "use-file-picker";

import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../..";

import { withoutUnsafeUrls } from "../plugins/safe-url-kit";
import { ToolbarButton } from "./toolbar";

type ImportType = "html" | "markdown";

/**
 * Parses into an inert DOMParser document, so markup never touches the live
 * page; accepts any HTML file, not only Plate exports.
 */
export function deserializeHtmlFile(editor: SlateEditor, html: string) {
	const document = parseHtmlDocument(html);
	const root =
		document.querySelector<HTMLElement>('[data-slate-editor="true"]') ??
		document.body;
	for (const element of root.querySelectorAll("script, style")) {
		element.remove();
	}
	return withoutUnsafeUrls(
		editor,
		editor.api.html.deserialize({ element: root }),
	);
}

export function deserializeMarkdownFile(editor: SlateEditor, markdown: string) {
	return editor.getApi(MarkdownPlugin).markdown.deserialize(markdown);
}

export function ImportToolbarButton(props: DropdownMenuProps) {
	const { t } = useTranslation("common");
	const editor = useEditorRef();
	const [open, setOpen] = React.useState(false);

	const getFileNodes = (text: string, type: ImportType) => {
		if (type === "html") {
			return deserializeHtmlFile(editor, text);
		}

		if (type === "markdown") {
			return deserializeMarkdownFile(editor, text);
		}

		return [];
	};

	const { openFilePicker: openMdFilePicker } = useFilePicker({
		accept: [".md", ".mdx"],
		multiple: false,
		onFilesSelected: async ({ plainFiles }: any) => {
			const text = await plainFiles[0].text();

			const nodes = getFileNodes(text, "markdown");

			editor.tf.insertNodes(nodes);
		},
	});

	const { openFilePicker: openHtmlFilePicker } = useFilePicker({
		accept: ["text/html"],
		multiple: false,
		onFilesSelected: async ({ plainFiles }: any) => {
			const text = await plainFiles[0].text();

			const nodes = getFileNodes(text, "html");

			editor.tf.insertNodes(nodes);
		},
	});

	return (
		<DropdownMenu open={open} onOpenChange={setOpen} modal={false} {...props}>
			<DropdownMenuTrigger asChild>
				<ToolbarButton pressed={open} tooltip="Import" isDropdown>
					<ArrowUpToLineIcon className="size-4" />
				</ToolbarButton>
			</DropdownMenuTrigger>

			<DropdownMenuContent align="start">
				<DropdownMenuGroup>
					<DropdownMenuItem
						onSelect={() => {
							openHtmlFilePicker();
						}}
					>
						{`Import from HTML`}
					</DropdownMenuItem>

					<DropdownMenuItem
						onSelect={() => {
							openMdFilePicker();
						}}
					>
						{`Import from Markdown`}
					</DropdownMenuItem>
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
