"use client";

import { IconBinary, IconPdf } from "@tabler/icons-react";
import {
	BracesIcon,
	Database,
	FileArchive,
	FileAudioIcon,
	FileIcon,
	FileImageIcon,
	FileSpreadsheetIcon,
	FileTextIcon,
	FileVideoIcon,
	Music,
	PresentationIcon,
	Settings,
	Zap,
} from "lucide-react";
import { Badge } from "./badge";
import { isAudio, isCode, isImage, isText, isVideo } from "./file-previewer";

/** One icon and one label per file type, shared by every surface that lists files. */
export function FileTypeIcon({
	name,
	className,
}: Readonly<{ name: string; className?: string }>) {
	const extension = name.split(".").pop()?.toLowerCase();

	if (name.toLowerCase().endsWith(".pdf"))
		return <IconPdf className={className} />;
	if (isImage(name)) return <FileImageIcon className={className} />;
	if (isVideo(name)) return <FileVideoIcon className={className} />;
	if (isAudio(name)) return <FileAudioIcon className={className} />;
	if (isCode(name)) return <BracesIcon className={className} />;
	if (isText(name)) return <FileTextIcon className={className} />;

	switch (extension) {
		case "zip":
		case "rar":
		case "7z":
		case "tar":
		case "gz":
			return <FileArchive className={className} />;
		case "xlsx":
		case "xls":
		case "csv":
			return <FileSpreadsheetIcon className={className} />;
		case "pptx":
		case "ppt":
			return <PresentationIcon className={className} />;
		case "sql":
		case "db":
		case "sqlite":
			return <Database className={className} />;
		case "json":
		case "xml":
		case "yaml":
		case "yml":
			return <BracesIcon className={className} />;
		case "exe":
		case "msi":
		case "app":
		case "deb":
		case "rpm":
			return <Zap className={className} />;
		case "conf":
		case "config":
		case "ini":
		case "env":
			return <Settings className={className} />;
		case "mp3":
		case "wav":
		case "flac":
		case "aac":
			return <Music className={className} />;
		default:
			if (!name.split("/").pop()?.includes("."))
				return <IconBinary className={className} />;
			return <FileIcon className={className} />;
	}
}

/** The file's category, or null when only its extension is known. */
export function fileTypeCategory(name: string): string | null {
	const extension = name.split(".").pop()?.toLowerCase();

	if (isImage(name)) return "Image";
	if (isVideo(name)) return "Video";
	if (isAudio(name)) return "Audio";
	if (isCode(name)) return "Code";
	if (isText(name)) return "Text";

	switch (extension) {
		case "pdf":
			return "PDF";
		case "zip":
		case "rar":
		case "7z":
		case "tar":
		case "gz":
			return "Archive";
		case "xlsx":
		case "xls":
		case "csv":
			return "Sheet";
		case "pptx":
		case "ppt":
			return "Slides";
		case "sql":
		case "db":
		case "sqlite":
			return "DB";
		case "json":
		case "xml":
		case "yaml":
		case "yml":
			return "Data";
		case "exe":
		case "msi":
		case "app":
		case "deb":
		case "rpm":
			return "Exec";
		default:
			return null;
	}
}

export function FileTypeBadge({
	filename,
	className,
}: Readonly<{ filename: string; className?: string }>) {
	const category = fileTypeCategory(filename);

	return (
		<Badge variant={category ? "secondary" : "outline"} className={className}>
			{category ?? filename.split(".").pop()?.toUpperCase() ?? "File"}
		</Badge>
	);
}
