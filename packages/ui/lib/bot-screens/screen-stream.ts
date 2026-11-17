/** A byte stream carrying the playground protocol; `DeviceServiceStream` satisfies it structurally. */
export interface ScreenStream {
	/** The next chunk the device sent; null once the stream ended. */
	read(): Promise<Uint8Array | null>;
	write(bytes: Uint8Array): Promise<void>;
	reset(): void;
}
