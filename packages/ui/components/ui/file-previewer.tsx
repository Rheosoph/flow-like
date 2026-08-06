"use client";
import { useCallback, useEffect, useState } from "react";
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
	const [content, setContent] = useState<string>("");
	const [pdfPreviewUrl, setPdfPreviewUrl] = useState("");
	const [pdfPreviewError, setPdfPreviewError] = useState<string | null>(null);

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

	useEffect(() => {
		if (!isPdf(url, filename)) {
			setPdfPreviewUrl("");
			setPdfPreviewError(null);
			return;
		}

		let cancelled = false;
		let objectUrl: string | null = null;

		setPdfPreviewUrl("");
		setPdfPreviewError(null);

		const loadPdf = async () => {
			try {
				const response = await fetch(url);
				if (!response.ok) {
					throw new Error("Failed to fetch PDF");
				}

				const blob = await response.blob();
				const pdfBlob =
					blob.type === "application/pdf"
						? blob
						: new Blob([blob], { type: "application/pdf" });
				objectUrl = URL.createObjectURL(pdfBlob);

				if (cancelled) {
					URL.revokeObjectURL(objectUrl);
					return;
				}

				setPdfPreviewUrl(objectUrl);
			} catch (error) {
				if (cancelled) return;
				console.error("Failed to load PDF preview:", error);
				setPdfPreviewError("Failed to load PDF preview");
			}
		};

		void loadPdf();

		return () => {
			cancelled = true;
			if (objectUrl) {
				URL.revokeObjectURL(objectUrl);
			}
		};
	}, [filename, url]);

	if (!canPreview(url, filename)) {
		return (
			<div className="text-red-500">File type not supported for preview</div>
		);
	}

	if (isPdf(url, filename)) {
		const pageUrl = page
			? `#page=${page}&#toolbar=1&#view=FitH`
			: "#toolbar=1&#view=FitH";

		if (pdfPreviewError) {
			return <div className="text-red-500">{pdfPreviewError}</div>;
		}

		if (!pdfPreviewUrl) {
			return (
				<div className="text-muted-foreground">Loading PDF preview...</div>
			);
		}

		return (
			<div className="w-full h-full flex flex-col">
				<iframe
					src={`${pdfPreviewUrl}${pageUrl}`}
					className="w-full h-full border-0 max-h-full max-w-full"
					title={`PDF Preview: ${rawFileName(url, filename)}`}
				>
					<p>
						Your browser cannot display the PDF.{" "}
						<a href={url} target="_blank" rel="noopener noreferrer">
							Download the PDF
						</a>{" "}
						instead.
					</p>
				</iframe>
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
		<div className="text-red-500">File type not supported for preview</div>
	);
}
