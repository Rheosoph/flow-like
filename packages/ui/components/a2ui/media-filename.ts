export function inferFileName(src: string, filename?: string): string {
	if (filename) return filename;

	try {
		const parsed = new URL(src, "https://flow-like.local/");
		const queryFilename = parsed.searchParams.get("filename");
		if (queryFilename) return queryFilename;
	} catch {}

	if (src.startsWith("data:")) {
		const mediaType = src.split(";")[0].split(":")[1];
		if (mediaType) {
			const extension = mediaType.split("/")[1];
			if (extension) return `file.${extension}`;
		}
		return "file";
	}

	const inferred = src.split(/[?#]/)[0]?.split("/").filter(Boolean).pop();
	if (!inferred) return "file";

	try {
		return decodeURIComponent(inferred);
	} catch {
		return inferred;
	}
}
