export interface PrivateClipboardValue {
	format: "text" | "html" | "image";
	text?: string;
	html?: string;
	imageBase64?: string;
	appPrivate: true;
	expiresAt: number;
}

/** HTML stays an attachment. It is never rendered in the clipboard dialog. */
export function privateClipboardDraft(value: PrivateClipboardValue): {
	text: string;
	attachments: File[];
} {
	if (
		value.appPrivate !== true ||
		!Number.isFinite(value.expiresAt) ||
		value.expiresAt <= Date.now()
	)
		throw new Error("This private copy has expired. Copy it again.");
	if (value.format === "text" && typeof value.text === "string")
		return { text: value.text, attachments: [] };
	if (value.format === "html" && typeof value.html === "string")
		return {
			text: value.text ?? "",
			attachments: [
				new File([value.html], "private-copy.html", { type: "text/html" }),
			],
		};
	if (value.format === "image" && typeof value.imageBase64 === "string") {
		if (value.imageBase64.length > 3 * 1024 * 1024)
			throw new Error("The private image is too large.");
		const bytes = Uint8Array.from(atob(value.imageBase64), (byte) =>
			byte.charCodeAt(0),
		);
		if (
			![137, 80, 78, 71, 13, 10, 26, 10].every(
				(byte, index) => bytes[index] === byte,
			)
		)
			throw new Error("The private image is not a PNG.");
		return {
			text: "",
			attachments: [
				new File([bytes], "private-copy.png", { type: "image/png" }),
			],
		};
	}
	throw new Error("This private clipboard item cannot be pasted.");
}
