export const ROOM_IMAGE_ACCEPT = "image/png,image/jpeg,image/webp,image/gif";

const ACCEPTED_TYPES = new Set(ROOM_IMAGE_ACCEPT.split(","));
const ROOM_IMAGE_EDGE = 256;
const MAX_SOURCE_BYTES = 20 * 1024 * 1024;
/** The server takes image data URLs of at most 512 KiB. */
const MAX_DATA_URL_LENGTH = 512 * 1024;

export type RoomImageProblem = "format" | "size" | "unreadable";

export class RoomImageError extends Error {
	constructor(readonly problem: RoomImageProblem) {
		super(`The room picture can't be used: ${problem}`);
		this.name = "RoomImageError";
	}
}

/** The square a centered crop of a `width` × `height` image takes, scaled to at most `edge`. */
export function centeredSquare(width: number, height: number, edge: number) {
	const side = Math.min(width, height);
	return {
		sx: (width - side) / 2,
		sy: (height - side) / 2,
		side,
		edge: Math.max(1, Math.min(edge, side)),
	};
}

/** The picked image cropped to its centered square and scaled to at most 256 px, as a data URL. */
export async function prepareRoomImage(file: Blob): Promise<string> {
	if (!ACCEPTED_TYPES.has(file.type)) throw new RoomImageError("format");
	if (file.size === 0 || file.size > MAX_SOURCE_BYTES)
		throw new RoomImageError("size");
	let bitmap: ImageBitmap;
	try {
		bitmap = await createImageBitmap(file);
	} catch {
		throw new RoomImageError("unreadable");
	}
	try {
		const crop = centeredSquare(bitmap.width, bitmap.height, ROOM_IMAGE_EDGE);
		const canvas = document.createElement("canvas");
		canvas.width = crop.edge;
		canvas.height = crop.edge;
		const context = canvas.getContext("2d");
		if (!context) throw new RoomImageError("unreadable");
		context.imageSmoothingQuality = "high";
		context.drawImage(
			bitmap,
			crop.sx,
			crop.sy,
			crop.side,
			crop.side,
			0,
			0,
			crop.edge,
			crop.edge,
		);
		const data = canvas.toDataURL("image/webp", 0.9);
		if (data.length > MAX_DATA_URL_LENGTH) throw new RoomImageError("size");
		return data;
	} finally {
		bitmap.close();
	}
}
