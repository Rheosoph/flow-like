"use client";

import { useTranslation } from "@flow-like/locales";
import * as React from "react";

import type { DropdownMenuProps } from "@radix-ui/react-dropdown-menu";

import { MarkdownPlugin } from "@platejs/markdown";
import { ArrowDownToLineIcon } from "lucide-react";
import { useEditorRef } from "platejs/react";

import { toast } from "sonner";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../..";
import { useBackend } from "../../../state/backend-state";
import { serializeNativeDocument } from "../native-document";
import { getPlateEditorDocument } from "../plugins/discussion-kit";
import { preparePublicationValue } from "../publication";
import { renderPublicationHtml } from "../publication-html";
import { useEditorUpload } from "../upload-context";

import { ToolbarButton } from "./toolbar";

export function ExportToolbarButton(props: DropdownMenuProps) {
	const { t } = useTranslation("common");
	const editor = useEditorRef();
	const backend = useBackend();
	const upload = useEditorUpload();
	const [open, setOpen] = React.useState(false);

	const getCanvas = async () => {
		const { default: html2canvas } = await import("html2canvas-pro");

		const editorNode = editor.api.toDOMNode(editor);
		if (!editorNode)
			throw new Error("The editor is not available for image export.");

		return html2canvas(editorNode, {
			onclone: (document: Document) => {
				const editorElement = document.querySelector(
					'[contenteditable="true"]',
				);
				if (editorElement) {
					for (const element of editorElement.querySelectorAll("*")) {
						const existingStyle = element.getAttribute("style") || "";
						element.setAttribute(
							"style",
							`${existingStyle}; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif !important`,
						);
					}
				}
			},
		});
	};

	const downloadFile = async (url: string, filename: string) => {
		const response = await fetch(url);

		const blob = await response.blob();
		const blobUrl = window.URL.createObjectURL(blob);

		const link = document.createElement("a");
		link.href = blobUrl;
		link.download = filename;
		document.body.append(link);
		link.click();
		link.remove();

		// Clean up the blob URL
		window.URL.revokeObjectURL(blobUrl);
	};

	const publicationHtml = async () =>
		renderPublicationHtml(
			await preparePublicationValue(
				editor.children,
				upload,
				backend.storageState,
			),
		);

	const exportToPdf = async () => {
		const printWindow = window.open("", "_blank");
		if (!printWindow) {
			toast.error("Allow pop-ups to open the print preview.");
			return;
		}
		try {
			const html = await publicationHtml();
			printWindow.document.open();
			printWindow.document.write(html);
			printWindow.document.close();
			await Promise.all(
				Array.from(printWindow.document.images).map((image) =>
					image.decode().catch(() => {}),
				),
			);
			await printWindow.document.fonts.ready;
			printWindow.focus();
			printWindow.print();
		} catch (error) {
			printWindow.close();
			toast.error(error instanceof Error ? error.message : "Export failed.");
		}
	};

	const exportToImage = async () => {
		try {
			const canvas = await getCanvas();
			await downloadFile(canvas.toDataURL("image/png"), "document.png");
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : "Image export failed.",
			);
		}
	};

	const exportToHtml = async () => {
		try {
			const html = await publicationHtml();
			await downloadFile(
				`data:text/html;charset=utf-8,${encodeURIComponent(html)}`,
				"document.html",
			);
		} catch (error) {
			toast.error(error instanceof Error ? error.message : "Export failed.");
		}
	};

	const exportToDocx = async () => {
		try {
			const { exportDocx } = await import("../docx");
			const value = await preparePublicationValue(
				editor.children,
				upload,
				backend.storageState,
			);
			const bytes = exportDocx(value);
			const url = URL.createObjectURL(
				new Blob([bytes as BlobPart], {
					type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
				}),
			);
			try {
				await downloadFile(url, "document.docx");
			} finally {
				URL.revokeObjectURL(url);
			}
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : "DOCX export failed.",
			);
		}
	};

	const exportNative = async () => {
		const document = serializeNativeDocument(getPlateEditorDocument(editor));
		await downloadFile(
			`data:application/json;charset=utf-8,${encodeURIComponent(document)}`,
			"document.plate.json",
		);
	};

	const exportToMarkdown = async () => {
		try {
			const value = await preparePublicationValue(
				editor.children,
				upload,
				backend.storageState,
			);
			const md = editor.getApi(MarkdownPlugin).markdown.serialize({ value });
			const url = `data:text/markdown;charset=utf-8,${encodeURIComponent(md)}`;
			await downloadFile(url, "document.md");
		} catch (error) {
			toast.error(error instanceof Error ? error.message : "Export failed.");
		}
	};

	return (
		<DropdownMenu open={open} onOpenChange={setOpen} modal={false} {...props}>
			<DropdownMenuTrigger asChild>
				<ToolbarButton pressed={open} tooltip="Export" isDropdown>
					<ArrowDownToLineIcon className="size-4" />
				</ToolbarButton>
			</DropdownMenuTrigger>

			<DropdownMenuContent align="start">
				<DropdownMenuGroup>
					<DropdownMenuItem onSelect={exportToDocx}>
						Export as Word (.docx)
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={exportNative}>
						Export editable document (.plate.json)
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={exportToHtml}>
						{t("exportAsHtml", "Export as HTML")}
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={exportToPdf}>
						Print / Save as PDF
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={exportToImage}>
						{t("exportAsImage", "Export as Image")}
					</DropdownMenuItem>
					<DropdownMenuItem onSelect={exportToMarkdown}>
						{t("exportAsMarkdown", "Export as Markdown")}
					</DropdownMenuItem>
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
