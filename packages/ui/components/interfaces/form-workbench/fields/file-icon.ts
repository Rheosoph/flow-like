import {
	FileArchive,
	FileAudio,
	FileJson,
	FileSpreadsheet,
	FileText,
	FileVideo,
	Image,
	type LucideIcon,
} from "lucide-react";

const BY_EXTENSION: Readonly<Record<string, LucideIcon>> = {
	png: Image,
	jpg: Image,
	jpeg: Image,
	gif: Image,
	webp: Image,
	svg: Image,
	heic: Image,
	csv: FileSpreadsheet,
	tsv: FileSpreadsheet,
	xls: FileSpreadsheet,
	xlsx: FileSpreadsheet,
	json: FileJson,
	mp4: FileVideo,
	mov: FileVideo,
	webm: FileVideo,
	mkv: FileVideo,
	mp3: FileAudio,
	wav: FileAudio,
	m4a: FileAudio,
	flac: FileAudio,
	zip: FileArchive,
	gz: FileArchive,
	tar: FileArchive,
	"7z": FileArchive,
};

/** A Lucide outline icon for a file name; documents and anything unknown read as a page of text. */
export function fileIconOf(name: string): LucideIcon {
	const dot = name.lastIndexOf(".");
	const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
	return BY_EXTENSION[extension] ?? FileText;
}
