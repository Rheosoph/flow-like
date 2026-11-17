import type {
	BarcodeDetect,
	DetectedBarcode,
} from "../interfaces/chat-default/chatbox/barcode-frames";

interface NativeDetector {
	detect(image: ImageData): Promise<
		{
			rawValue: string;
			format: string;
			cornerPoints?: { x: number; y: number }[];
		}[]
	>;
}

type NativeDetectorClass = new (options: {
	formats: string[];
}) => NativeDetector;

/** QR codes through the browser's own `BarcodeDetector`; undefined where it has none. */
export function browserBarcodeDetect(): BarcodeDetect | undefined {
	const Detector = (globalThis as { BarcodeDetector?: NativeDetectorClass })
		.BarcodeDetector;
	if (!Detector || typeof ImageData === "undefined") return undefined;
	const detector = new Detector({ formats: ["qr_code"] });
	return async (frame) => {
		const rgba = new Uint8ClampedArray(frame.width * frame.height * 4);
		for (let index = 0; index < frame.luma.length; index += 1) {
			const value = frame.luma[index];
			rgba.set([value, value, value, 255], index * 4);
		}
		const found = await detector.detect(
			new ImageData(rgba, frame.width, frame.height),
		);
		return found.map(
			(code): DetectedBarcode => ({
				text: code.rawValue,
				format: code.format,
				points: code.cornerPoints ?? [],
			}),
		);
	};
}
