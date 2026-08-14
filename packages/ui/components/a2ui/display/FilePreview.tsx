"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { cn } from "../../../lib/utils";
import { AudioPreview } from "../../ui/audio-preview";
import { PdfFrame } from "../../ui/file-previewer";
import { AudioPlayback, type VoiceVariant } from "../../voice";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, FilePreviewComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function rawFileName(url: string, filename?: string): string {
	if (filename) return filename;

	try {
		const parsed = new URL(url, window.location.href);
		const queryFilename = parsed.searchParams.get("filename");
		if (queryFilename) return queryFilename;
	} catch {}

	if (url.startsWith("data:")) {
		const mediaType = url.split(";")[0].split(":")[1];
		if (mediaType) {
			const extension = mediaType.split("/")[1];
			if (extension) return `file.${extension}`;
		}
		return "file";
	}
	return url.split("?")[0].split("/").pop() ?? "";
}

function getFileType(
	url: string,
	mimeType?: string,
	filename?: string,
	fileType?: string,
): "pdf" | "image" | "video" | "audio" | "code" | "text" | "unknown" {
	if (
		fileType === "pdf" ||
		fileType === "image" ||
		fileType === "video" ||
		fileType === "audio" ||
		fileType === "code" ||
		fileType === "text"
	) {
		return fileType;
	}

	const normalizedMime = mimeType?.toLowerCase() ?? "";
	if (normalizedMime === "application/pdf") return "pdf";
	if (normalizedMime.startsWith("image/")) return "image";
	if (normalizedMime.startsWith("video/")) return "video";
	if (normalizedMime.startsWith("audio/")) return "audio";
	if (normalizedMime.startsWith("text/")) return "text";
	if (normalizedMime.includes("json") || normalizedMime.includes("javascript"))
		return "code";

	const name = rawFileName(url, filename).toLowerCase();
	if (/\.(pdf)$/i.test(name)) return "pdf";
	if (/\.(png|jpg|jpeg|gif|bmp|webp|svg)$/i.test(name)) return "image";
	if (/\.(mp4|mkv|webm|ogv|avi|mov)$/i.test(name)) return "video";
	if (/\.(mp3|wav|ogg|oga|opus|flac|aac|m4a|aif|aiff)$/i.test(name))
		return "audio";
	if (
		/\.(json|xml|css|js|jsx|ts|tsx|py|java|c|cpp|h|hpp|cs|go|rb|php|swift|kt|rs|html|yml|yaml|toml|sql|sh|bash|scss|sass|less|vue|svelte)$/i.test(
			name,
		)
	)
		return "code";
	if (/\.(txt|csv|md|mdx|ini|conf|cfg|log|env)$/i.test(name)) return "text";
	return "unknown";
}

function getCodeLanguage(file: string, filename?: string): string {
	const match =
		/\.(json|xml|css|js|jsx|ts|tsx|py|java|c|cpp|h|hpp|cs|go|rb|php|swift|kt|rs|html|yml|yaml|toml|sql|sh|bash|scss|sass|less|vue|svelte)$/i.exec(
			rawFileName(file, filename),
		);
	return match?.[0]?.replace(".", "") ?? "text";
}

export function A2UIFilePreview({
	component,
	style,
}: ComponentProps<FilePreviewComponent>) {
	const { t } = useTranslation("common");
	const src = useResolved<string>(component.src ?? component.url);
	const filename = useResolved<string>(component.filename);
	const mimeType = useResolved<string>(component.mimeType);
	const fileTypeOverride = useResolved<string>(component.fileType);
	const showControls = useResolved<boolean>(component.showControls) ?? true;
	const showDownload = useResolved<boolean>(component.showDownload) ?? false;
	const audioVariant = useResolved<string>(component.variant);
	const audioAutoPlay = useResolved<boolean>(component.autoPlay) ?? false;
	const fit = useResolved<string>(component.fit) ?? "contain";
	const loading = useResolved<"lazy" | "eager">(component.loading);
	const fallbackText =
		useResolved<string>(component.fallbackText) ?? t('cannotPreviewThisFile', 'Cannot preview this file');

	const [content, setContent] = useState<string>("");
	const [loadingText, setLoadingText] = useState(false);
	const [error, setError] = useState(false);

	const fileType = src
		? getFileType(src, mimeType, filename, fileTypeOverride)
		: "unknown";

	const loadTextContent = useCallback(async () => {
		if (!src) return;
		setLoadingText(true);
		setError(false);
		setContent("");
		try {
			const response = await fetch(src);
			if (!response.ok) throw new Error("Failed to fetch");
			setContent(await response.text());
		} catch {
			setError(true);
		} finally {
			setLoadingText(false);
		}
	}, [src]);

	useEffect(() => {
		if (fileType === "code" || fileType === "text") {
			loadTextContent();
		} else {
			setContent("");
			setLoadingText(false);
			setError(false);
		}
	}, [fileType, loadTextContent]);

	const fitClass =
		{
			contain: "object-contain",
			cover: "object-cover",
			fill: "object-fill",
			none: "object-none",
			scaleDown: "object-scale-down",
		}[fit] ?? "object-contain";

	if (!src || error) {
		return (
			<div
				className={cn(
					"flex items-center justify-center text-muted-foreground p-4",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				{fallbackText}
			</div>
		);
	}

	if (fileType === "pdf") {
		return (
			<div
				className={cn("w-full h-full flex flex-col", resolveStyle(style))}
				style={resolveInlineStyle(style)}
			>
				<PdfFrame
					url={src}
					filename={filename ?? rawFileName(src)}
					loading={loading ?? "lazy"}
				/>
			</div>
		);
	}

	if (fileType === "image") {
		return (
			<img
				src={src}
				alt={rawFileName(src, filename)}
				className={cn("w-full h-full", fitClass, resolveStyle(style))}
				style={resolveInlineStyle(style)}
				loading={loading ?? "lazy"}
				onError={() => setError(true)}
			/>
		);
	}

	if (fileType === "video") {
		return (
			<video
				src={src}
				controls={showControls}
				preload={loading === "eager" ? "auto" : "metadata"}
				className={cn("w-full h-full", fitClass, resolveStyle(style))}
				style={resolveInlineStyle(style)}
			><Trans i18nKey="trackKindcaptionsSrclangenLabelenglishCaptionsYourBrowserDoesNotSupportTheVideoTag"><track kind="captions" srcLang="en" label="English captions" />
				Your browser does not support the video tag.</Trans></video>
		);
	}

	if (fileType === "audio") {
		if (audioVariant) {
			return (
				<div
					className={cn(
						"flex h-full w-full items-center justify-center p-4",
						resolveStyle(style),
					)}
					style={resolveInlineStyle(style)}
				>
					<div className="w-full max-w-2xl rounded-xl border bg-card p-5 shadow-sm">
						<AudioPlayback
							src={src}
							variant={audioVariant as VoiceVariant}
							title={rawFileName(src, filename)}
							autoPlay={audioAutoPlay}
							downloadName={
								showDownload ? rawFileName(src, filename) : undefined
							}
						/>
					</div>
				</div>
			);
		}
		return (
			<AudioPreview
				src={src}
				title={rawFileName(src, filename)}
				mimeType={mimeType}
				showControls={showControls}
				showDownload={showDownload}
				className={resolveStyle(style)}
				style={resolveInlineStyle(style)}
				onError={() => setError(true)}
			/>
		);
	}

	if (fileType === "code" || fileType === "text") {
		const lang = fileType === "code" ? getCodeLanguage(src, filename) : "";
		return (
			<div
				className={cn(
					"w-full h-full overflow-auto bg-muted/30 rounded",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				<pre className="p-4 text-sm font-mono whitespace-pre-wrap break-all">
					{lang && (
						<div className="text-xs text-muted-foreground mb-2 uppercase">
							{lang}
						</div>
					)}
					<code>{loadingText ? "Loading..." : content}</code>
				</pre>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex items-center justify-center text-muted-foreground p-4",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			{fallbackText}
		</div>
	);
}
