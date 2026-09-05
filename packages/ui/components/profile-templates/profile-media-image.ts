export type ProfileMediaKind = "icon" | "cover";

export const PROFILE_MEDIA_MAX_BYTES = 10 * 1024 * 1024;
export const PROFILE_MEDIA_MAX_EDGE = 4096;
export const PROFILE_MEDIA_ACCEPT = "image/png,image/jpeg,image/webp";

const IMAGE_TYPES = new Set([
	"image/png",
	"image/jpeg",
	"image/jpg",
	"image/webp",
]);
const FORMAT_ERROR = "Choose a PNG, JPEG, or WebP image.";

export function profileMediaDimensions(bytes: Uint8Array): {
	width: number;
	height: number;
} {
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	const ascii = (start: number, length: number) =>
		String.fromCharCode(...bytes.subarray(start, start + length));
	if (
		bytes.length >= 24 &&
		bytes[0] === 0x89 &&
		ascii(1, 3) === "PNG" &&
		bytes[4] === 13 &&
		bytes[5] === 10 &&
		bytes[6] === 26 &&
		bytes[7] === 10 &&
		ascii(12, 4) === "IHDR"
	) {
		return { width: view.getUint32(16), height: view.getUint32(20) };
	}
	if (bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
		let offset = 2;
		while (offset + 3 < bytes.length) {
			if (bytes[offset++] !== 0xff) break;
			while (bytes[offset] === 0xff) offset++;
			const marker = bytes[offset++];
			if (marker === 0xda || marker === 0xd9) break;
			if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
			if (offset + 2 > bytes.length) break;
			const length = view.getUint16(offset);
			if (length < 2 || offset + length > bytes.length) break;
			if (
				length >= 7 &&
				marker >= 0xc0 &&
				marker <= 0xcf &&
				![0xc4, 0xc8, 0xcc].includes(marker)
			) {
				return {
					height: view.getUint16(offset + 3),
					width: view.getUint16(offset + 5),
				};
			}
			offset += length;
		}
	}
	if (bytes.length >= 25 && ascii(0, 4) === "RIFF" && ascii(8, 4) === "WEBP") {
		const chunk = ascii(12, 4);
		if (chunk === "VP8X" && bytes.length >= 30) {
			return {
				width: 1 + bytes[24] + (bytes[25] << 8) + (bytes[26] << 16),
				height: 1 + bytes[27] + (bytes[28] << 8) + (bytes[29] << 16),
			};
		}
		if (
			chunk === "VP8 " &&
			bytes.length >= 30 &&
			bytes[23] === 0x9d &&
			bytes[24] === 1 &&
			bytes[25] === 0x2a
		) {
			return {
				width: view.getUint16(26, true) & 0x3fff,
				height: view.getUint16(28, true) & 0x3fff,
			};
		}
		if (chunk === "VP8L" && bytes[20] === 0x2f) {
			return {
				width: 1 + bytes[21] + ((bytes[22] & 0x3f) << 8),
				height:
					1 + (bytes[22] >> 6) + (bytes[23] << 2) + ((bytes[24] & 0x0f) << 10),
			};
		}
	}
	throw new Error(
		"This image could not be read. Choose another PNG, JPEG, or WebP file.",
	);
}

export function fitProfileMedia(
	width: number,
	height: number,
	kind: ProfileMediaKind,
) {
	if (
		!Number.isInteger(width) ||
		!Number.isInteger(height) ||
		width < 1 ||
		height < 1
	)
		throw new Error("This image has invalid dimensions.");
	if (width > PROFILE_MEDIA_MAX_EDGE || height > PROFILE_MEDIA_MAX_EDGE)
		throw new Error(
			"Choose an image no larger than 4096 pixels on either edge.",
		);
	const scale = Math.min(
		1,
		(kind === "icon" ? 512 : 1600) / Math.max(width, height),
	);
	return {
		width: Math.max(1, Math.round(width * scale)),
		height: Math.max(1, Math.round(height * scale)),
	};
}

export function profileMediaUrl(value: string): string | null {
	const url = value.trim();
	if (!url) return null;
	if (
		[...url].some(
			(char) =>
				char === "\\" || char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
		)
	)
		throw new Error("Enter an HTTP or HTTPS image URL.");
	try {
		const parsed = new URL(url);
		if (
			["http:", "https:"].includes(parsed.protocol) &&
			!parsed.username &&
			!parsed.password
		)
			return parsed.toString();
	} catch {}
	throw new Error("Enter an HTTP or HTTPS image URL.");
}

export async function prepareProfileMedia(
	file: Blob,
	kind: ProfileMediaKind,
): Promise<Blob> {
	if (file.type && !IMAGE_TYPES.has(file.type.toLowerCase()))
		throw new Error(FORMAT_ERROR);
	if (file.size === 0)
		throw new Error("This file is empty. Choose another image.");
	if (file.size > PROFILE_MEDIA_MAX_BYTES)
		throw new Error("Choose an image smaller than 10 MB.");
	const dimensions = profileMediaDimensions(
		new Uint8Array(await file.arrayBuffer()),
	);
	fitProfileMedia(dimensions.width, dimensions.height, kind);
	const objectUrl = URL.createObjectURL(file);
	const image = new Image();
	try {
		await new Promise<void>((resolve, reject) => {
			image.onload = () => resolve();
			image.onerror = () =>
				reject(
					new Error("This image could not be decoded. Choose another file."),
				);
			image.src = objectUrl;
		});
		const size = fitProfileMedia(image.naturalWidth, image.naturalHeight, kind);
		const canvas = document.createElement("canvas");
		canvas.width = size.width;
		canvas.height = size.height;
		const context = canvas.getContext("2d");
		if (!context) throw new Error("Your browser could not prepare this image.");
		context.imageSmoothingEnabled = true;
		context.imageSmoothingQuality = "high";
		context.drawImage(image, 0, 0, size.width, size.height);
		return await new Promise<Blob>((resolve, reject) => {
			canvas.toBlob(
				(blob) => {
					if (!blob || !["image/webp", "image/png"].includes(blob.type))
						reject(
							new Error(
								"Your browser could not prepare this image. Try another file.",
							),
						);
					else resolve(blob);
				},
				"image/webp",
				0.86,
			);
		});
	} finally {
		image.onload = null;
		image.onerror = null;
		image.src = "";
		URL.revokeObjectURL(objectUrl);
	}
}
