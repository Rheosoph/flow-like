"use client";
import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { cn } from "../../lib";
import { AudioPreview } from "./audio-preview";
import { MonacoFileEditor } from "./monaco-file-editor";
import { TextEditor } from "./text-editor";

function rawFileName(url: string, filename?: string) {
	if (filename) return filename;

	if (url.startsWith("data:")) {
		const mediaType = url.split(";")[0].split(":")[1];
		if (mediaType) {
			const extension = mediaType.split("/")[1];
			if (extension) {
				return `file.${extension}`;
			}
		}
		return "file";
	}

	return url.split("?")[0].split("/").pop() ?? "";
}

export function isPdf(file: string, filename?: string) {
	return /\.(pdf)$/i.test(rawFileName(file, filename));
}

export function isImage(file: string, filename?: string) {
	return /\.(png|jpg|jpeg|gif|bmp|webp|svg)$/i.test(
		rawFileName(file, filename),
	);
}

export function isVideo(file: string, filename?: string) {
	return /\.(mp4|mkv|webm|ogv|avi|mov)$/i.test(rawFileName(file, filename));
}

export function isAudio(file: string, filename?: string) {
	return /\.(mp3|wav|ogg|oga|opus|flac|aac|m4a|aif|aiff)$/i.test(
		rawFileName(file, filename),
	);
}

export function isCode(file: string, filename?: string) {
	return /\.(json|xml|css|js|jsx|ts|tsx|py|java|c|cpp|h|hpp|cs|go|rb|php|swift|kt|rs|html|yml|yaml|toml|sql|sh|bash|scss|sass|less|vue|svelte)$/i.test(
		rawFileName(file, filename),
	);
}

export function getCodeLanguage(file: string, filename?: string) {
	return (
		/\.(json|xml|css|js|jsx|ts|tsx|py|java|c|cpp|h|hpp|cs|go|rb|php|swift|kt|rs|html|yml|yaml|toml|sql|sh|bash|scss|sass|less|vue|svelte)$/i
			.exec(rawFileName(file, filename))?.[0]
			?.replaceAll(".", "") ?? "text"
	);
}

export function isText(file: string, filename?: string) {
	if (isCode(file, filename)) return true;
	return /\.(txt|csv|html|md|mdx|ini|conf|cfg|log|env)$/i.test(
		rawFileName(file, filename),
	);
}

export function canPreview(file: string, filename?: string) {
	return (
		isPdf(file, filename) ||
		isImage(file, filename) ||
		isVideo(file, filename) ||
		isAudio(file, filename) ||
		isText(file, filename)
	);
}

/**
 * PDF open parameters for the built-in viewers. Chrome cuts the fragment at the
 * second `#`, so every parameter has to sit behind a single one — `#page=2&#view=FitH`
 * silently drops the view.
 */
export function pdfViewerFragment(page?: number) {
	const params = ["toolbar=1", "view=FitH"];
	if (page && page > 0) params.unshift(`page=${Math.trunc(page)}`);
	return `#${params.join("&")}`;
}

type PdfSource = { src: string; state: "loading" | "blob" | "direct" };

/**
 * Re-types PDF bytes locally before handing them to the viewer.
 *
 * Nothing stamps a content type on upload, so signed storage URLs answer with
 * `binary/octet-stream`, and every browser hands anything that is not
 * `application/pdf` — or that carries `Content-Disposition: attachment` — to the
 * download manager instead of rendering it in the frame. A failed fetch (CORS,
 * custom asset protocols) falls back to the raw URL, so sources that render
 * today keep rendering.
 */
export function usePdfSource(url: string): PdfSource {
	const [source, setSource] = useState<PdfSource>({
		src: "",
		state: "loading",
	});

	useEffect(() => {
		let cancelled = false;
		let objectUrl: string | null = null;
		setSource({ src: "", state: "loading" });

		const load = async () => {
			try {
				const response = await fetch(url);
				if (!response.ok) {
					throw new Error(`Failed to fetch PDF (${response.status})`);
				}
				const blob = await response.blob();
				if (cancelled) return;

				const pdfBlob =
					blob.type === "application/pdf"
						? blob
						: new Blob([blob], { type: "application/pdf" });
				objectUrl = URL.createObjectURL(pdfBlob);
				setSource({ src: objectUrl, state: "blob" });
			} catch (error) {
				if (cancelled) return;
				console.warn("PDF preview fell back to the direct URL:", error);
				setSource({ src: url, state: "direct" });
			}
		};

		void load();

		return () => {
			cancelled = true;
			if (objectUrl) URL.revokeObjectURL(objectUrl);
		};
	}, [url]);

	return source;
}

export function PdfFrame({
	url,
	page,
	filename,
	className,
	loading,
}: Readonly<{
	url: string;
	page?: number;
	filename?: string;
	className?: string;
	loading?: "lazy" | "eager";
}>) {
	const { t } = useTranslation("common");
	const { src, state } = usePdfSource(url);

	if (state === "loading") {
		return (
			<div className="flex h-full w-full items-center justify-center p-4 text-sm text-muted-foreground">
				{t("loadingPdfPreview", "Loading PDF preview...")}
			</div>
		);
	}

	return (
		<iframe
			src={`${src.split("#")[0]}${pdfViewerFragment(page)}`}
			className={cn("w-full h-full border-0 max-h-full max-w-full", className)}
			title={t("pdfPreviewVal", "PDF Preview: {{val}}", {
				val: rawFileName(url, filename),
			})}
			loading={loading}
		>
			<p>
				{t(
					"yourBrowserCannotDisplayThePdf",
					"Your browser cannot display the PDF.",
				)}{" "}
				<a href={url} target="_blank" rel="noopener noreferrer">
					{t("downloadThePdf", "Download the PDF")}
				</a>{" "}
				instead.
			</p>
		</iframe>
	);
}

export function FilePreviewer({
	url,
	page,
	filename,
	mimeType,
	editable = false,
	onSave,
}: Readonly<{
	url: string;
	page?: number;
	filename?: string;
	mimeType?: string;
	editable?: boolean;
	onSave?: (content: string) => Promise<void>;
}>) {
	const { t } = useTranslation("common");
	const [content, setContent] = useState<string>("");

	const previewContent = useCallback(async () => {
		const response = await fetch(url);
		if (!response.ok) {
			throw new Error("Failed to fetch file");
		}
		setContent(await response.text());
	}, [url]);

	useEffect(() => {
		if (isText(url, filename)) {
			previewContent();
		}
	}, [filename, previewContent, url]);

	if (!canPreview(url, filename)) {
		return (
			<div className="text-red-500">
				{t(
					"fileTypeNotSupportedForPreview",
					"File type not supported for preview",
				)}
			</div>
		);
	}

	if (isPdf(url, filename)) {
		return (
			<div className="w-full h-full flex flex-col">
				<PdfFrame url={url} page={page} filename={filename} />
			</div>
		);
	}

	if (isImage(url, filename)) {
		return (
			<img
				src={url}
				alt={rawFileName(url, filename)}
				className="w-full h-full object-contain"
			/>
		);
	}

	if (isVideo(url, filename)) {
		return (
			<video src={url} controls className="w-full h-full object-contain">
				<track
					kind="captions"
					label="English captions"
					srcLang="en"
					src=""
					default={false}
				/>
				Your browser does not support the video tag.
			</video>
		);
	}

	if (isAudio(url, filename)) {
		return (
			<AudioPreview
				src={url}
				title={rawFileName(url, filename)}
				mimeType={mimeType}
				showDownload={true}
			/>
		);
	}

	if (isCode(url, filename)) {
		if (editable && onSave) {
			return (
				<MonacoFileEditor
					fileName={rawFileName(url, filename)}
					initialContent={content}
					editable={true}
					onSave={onSave}
				/>
			);
		}
		return (
			<TextEditor
				initialContent={`\n\`\`\`${getCodeLanguage(
					url,
					filename,
				)}\n${content}\n\`\`\`\n`}
				isMarkdown={true}
				editable={false}
			/>
		);
	}

	if (isText(url, filename)) {
		if (editable && onSave) {
			return (
				<MonacoFileEditor
					fileName={rawFileName(url, filename)}
					initialContent={content}
					editable={true}
					onSave={onSave}
				/>
			);
		}
		return (
			<TextEditor initialContent={content} isMarkdown={true} editable={false} />
		);
	}

	return (
		<div className="text-red-500">
			{t(
				"fileTypeNotSupportedForPreview",
				"File type not supported for preview",
			)}
		</div>
	);
}
