import { invoke } from "@tauri-apps/api/core";

/** Reads files the native layer placed in its shared-file inbox (share sheet, scanner, camera). */
export async function readNativeSharedFiles(paths: string[]): Promise<File[]> {
	if (paths.length > 10) throw new Error("Share at most ten files at a time.");
	const files: File[] = [];
	let totalBytes = 0;
	for (const path of paths) {
		const file = await invoke<{
			bytes: number[];
			name: string;
			mimeType?: string;
		}>("native_read_shared_file", { path });
		totalBytes += file.bytes.length;
		if (totalBytes > 35 * 1024 * 1024)
			throw new Error("Share at most 35 MiB of files at a time.");
		files.push(
			new File([new Uint8Array(file.bytes)], file.name, {
				type: file.mimeType || "application/octet-stream",
			}),
		);
	}
	return files;
}
