"use client";

import * as React from "react";

import type { DropdownMenuProps } from "@radix-ui/react-dropdown-menu";

import { ArrowUpToLineIcon } from "lucide-react";
import { type SlateEditor, parseHtmlDocument } from "platejs";
import { useEditorRef } from "platejs/react";
import { toast } from "sonner";
import { useFilePicker } from "use-file-picker";
import { deserializeNativeDocumentFile } from "../native-document";
import { applyPlateEditorDocument } from "../plugins/discussion-kit";

import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../..";

import { RICH_REMARK_PLUGINS, safeDeserialize } from "../../ui/text-editor";
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
	return safeDeserialize(editor, markdown, true, RICH_REMARK_PLUGINS);
}

export function ImportToolbarButton(props: DropdownMenuProps) {
	const editor = useEditorRef();
	const [open, setOpen] = React.useState(false);

	const { openFilePicker: openNativeFilePicker } = useFilePicker({
		accept: [".json"],
		multiple: false,
		onFilesSelected: async (result: { plainFiles?: File[] }) => {
			if (!result.plainFiles?.length) return;
			const { plainFiles } = result;
			if (!plainFiles[0]) return;
			try {
				const document = await deserializeNativeDocumentFile(plainFiles[0]);
				applyPlateEditorDocument(editor, document);
			} catch (error) {
				toast.error(error instanceof Error ? error.message : "Import failed.");
			}
		},
	});

	const { openFilePicker: openDocxFilePicker } = useFilePicker({
		accept: [".docx"],
		multiple: false,
		onFilesSelected: async (result: { plainFiles?: File[] }) => {
			if (!result.plainFiles?.[0]) return;
			try {
				const { importDocx } = await import("../docx");
				const resultDoc = importDocx(
					new Uint8Array(await result.plainFiles[0].arrayBuffer()),
				);
				editor.tf.insertNodes(withoutUnsafeUrls(editor, resultDoc.value));
				for (const warning of resultDoc.warnings) toast.warning(warning);
			} catch (error) {
				toast.error(
					error instanceof Error ? error.message : "DOCX import failed.",
				);
			}
		},
	});

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
		onFilesSelected: async (result: { plainFiles?: File[] }) => {
			if (!result.plainFiles?.[0]) return;
			const text = await result.plainFiles[0].text();

			const nodes = getFileNodes(text, "markdown");

			editor.tf.insertNodes(nodes);
		},
	});

	const { openFilePicker: openHtmlFilePicker } = useFilePicker({
		accept: ["text/html"],
		multiple: false,
		onFilesSelected: async (result: { plainFiles?: File[] }) => {
			if (!result.plainFiles?.[0]) return;
			const text = await result.plainFiles[0].text();

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
					<DropdownMenuItem onSelect={() => openDocxFilePicker()}>
						Import from Word (.docx)
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={() => openNativeFilePicker()}>
						Open editable document (.plate.json)
					</DropdownMenuItem>
					<DropdownMenuItem
						onSelect={() => {
							openHtmlFilePicker();
						}}
					>
						Import from HTML
					</DropdownMenuItem>

					<DropdownMenuItem
						onSelect={() => {
							openMdFilePicker();
						}}
					>
						Import from Markdown
					</DropdownMenuItem>
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
