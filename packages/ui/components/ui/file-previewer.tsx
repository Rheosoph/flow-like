"use client";
import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { cn } from "../../lib/utils";
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
	return /\.(png|apng|jpg|jpeg|jpe|jfif|gif|bmp|webp|svg)$/i.test(
		rawFileName(file, filename),
	);
}

export function isVideo(file: string, filename?: string) {
	return /\.(mp4|m4v|mkv|webm|ogv|avi|mov)$/i.test(rawFileName(file, filename));
}

export function isAudio(file: string, filename?: string) {
	return /\.(mp3|wav|wave|ogg|oga|opus|flac|aac|m4a|aif|aiff)$/i.test(
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
 * second `#`, so every parameter has to sit behind a single one. `#page=2&#view=FitH`
 * silently drops the view.
 */
export function pdfViewerFragment(page?: number) {
	const params = ["toolbar=1", "view=FitH"];
	if (page && page > 0) params.unshift(`page=${Math.trunc(page)}`);
	return `#${params.join("&")}`;
}

type PreviewSource = {
	src: string;
	state: "loading" | "blob" | "direct" | "error";
};

/**
 * Re-types bytes locally before handing them to the browser's native viewer.
 *
 * Storage URLs can return `binary/octet-stream` instead of the file's MIME type.
 * PDFs can download instead of rendering, and SVGs fail to display as images.
 * SVGs can fall back to their original URL. PDFs must not navigate to an
 * unverified response, since its headers can trigger a download.
 */
function useTypedPreviewSource(url: string, mimeType: string): PreviewSource {
	const [source, setSource] = useState<PreviewSource>({
		src: "",
		state: "loading",
	});

	useEffect(() => {
		let cancelled = false;
		const controller = new AbortController();
		let objectUrl: string | null = null;
		setSource({ src: "", state: "loading" });

		const load = async () => {
			try {
				const response = await fetch(url, {
					cache: "no-store",
					signal: controller.signal,
				});
				if (!response.ok) {
					throw new Error(`Failed to fetch preview (${response.status})`);
				}
				const blob = await response.blob();
				if (cancelled) return;

				const typedBlob =
					blob.type === mimeType ? blob : new Blob([blob], { type: mimeType });
				objectUrl = URL.createObjectURL(typedBlob);
				setSource({ src: objectUrl, state: "blob" });
			} catch (error) {
				if (cancelled) return;
				if (mimeType === "application/pdf") {
					let source = "invalid URL";
					try {
						const parsed = new URL(url, window.location.href);
						source = `${parsed.protocol} ${parsed.origin}`;
					} catch {}
					// Signed query strings can grant access to the attachment.
					console.warn("Failed to load PDF preview:", source, error);
					setSource({ src: "", state: "error" });
					return;
				}
				console.warn("File preview fell back to the direct URL:", error);
				setSource({ src: url, state: "direct" });
			}
		};

		void load();

		return () => {
			cancelled = true;
			controller.abort();
			if (objectUrl) URL.revokeObjectURL(objectUrl);
		};
	}, [url, mimeType]);

	return source;
}

export function usePdfSource(url: string): PreviewSource {
	return useTypedPreviewSource(url, "application/pdf");
}

function SvgPreview({ url, filename }: { url: string; filename?: string }) {
	const { t } = useTranslation("common");
	const { src, state } = useTypedPreviewSource(url, "image/svg+xml");
	if (state === "loading") {
		return (
			<div className="flex h-full w-full items-center justify-center p-4 text-sm text-muted-foreground">
				{t("loadingImagePreview", "Loading image preview...")}
			</div>
		);
	}
	return (
		<img
			src={src}
			alt={rawFileName(url, filename)}
			className="w-full h-full object-contain"
		/>
	);
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
	const [attempt, setAttempt] = useState(0);

	return (
		<NativePdfFrame
			key={`${url}\n${attempt}`}
			url={url}
			page={page}
			filename={filename}
			className={className}
			loading={loading}
			onRetry={() => setAttempt((value) => value + 1)}
		/>
	);
}

function NativePdfFrame({
	url,
	page,
	filename,
	className,
	loading,
	onRetry,
}: {
	url: string;
	page?: number;
	filename?: string;
	className?: string;
	loading?: "lazy" | "eager";
	onRetry: () => void;
}) {
	const { t } = useTranslation("common");
	const { src, state } = usePdfSource(url);

	if (state === "error") {
		return (
			<div className="flex h-full w-full flex-col items-center justify-center gap-3 p-4 text-sm">
				<p role="alert">
					{t("failedToLoadPdfPreview", "Could not load the PDF preview.")}
				</p>
				<button type="button" className="underline" onClick={onRetry}>
					{t("retry", "Retry")}
				</button>
				<a
					href={url}
					download={filename || "document.pdf"}
					target="_blank"
					rel="noopener noreferrer"
					className="underline"
				>
					{t("downloadThePdf", "Download the PDF")}
				</a>
			</div>
		);
	}

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
		if (/\.svg$/i.test(rawFileName(url, filename))) {
			return <SvgPreview url={url} filename={filename} />;
		}
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
