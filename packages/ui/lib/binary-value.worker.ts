import { type BinaryDecodeOptions, decodeBinary } from "./binary-value";

interface BinaryDecodeRequest {
	id: number;
	bytes: Uint8Array;
	options: BinaryDecodeOptions;
}

self.onmessage = async (event: MessageEvent<BinaryDecodeRequest>) => {
	const { id, bytes, options } = event.data;
	try {
		const reading = await decodeBinary(bytes, options);
		const transfer = reading.kind === "binary" ? [reading.head.buffer] : [];
		self.postMessage({ id, ok: true, reading }, { transfer });
	} catch (error) {
		self.postMessage({ id, ok: false, error: String(error) });
	}
};
