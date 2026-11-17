import type { ScreenStream } from "./screen-stream";

export const MAX_RECORD_PLAINTEXT = 16_368;
const TAG_BYTES = 16;
const encoder = new TextEncoder();
const INFO_PREFIX = "flow-like/bot-screens/stream/v1|";

export interface StreamKeys {
	viewerToHost: CryptoKey;
	hostToViewer: CryptoKey;
}

async function derive(
	material: CryptoKey,
	session: string,
	direction: string,
): Promise<CryptoKey> {
	return crypto.subtle.deriveKey(
		{
			name: "HKDF",
			hash: "SHA-256",
			salt: encoder.encode(session) as BufferSource,
			info: encoder.encode(`${INFO_PREFIX}${direction}`) as BufferSource,
		},
		material,
		{ name: "AES-GCM", length: 256 },
		true,
		["encrypt", "decrypt"],
	);
}

export async function deriveStreamKeys(
	viewerSecret: Uint8Array,
	hostSecret: Uint8Array,
	session: string,
): Promise<StreamKeys> {
	const ikm = new Uint8Array(viewerSecret.length + hostSecret.length);
	ikm.set(viewerSecret);
	ikm.set(hostSecret, viewerSecret.length);
	const material = await crypto.subtle.importKey(
		"raw",
		ikm as BufferSource,
		"HKDF",
		false,
		["deriveKey"],
	);
	return {
		viewerToHost: await derive(material, session, "viewer->host"),
		hostToViewer: await derive(material, session, "host->viewer"),
	};
}

function nonce(counter: bigint): Uint8Array {
	const bytes = new Uint8Array(12);
	new DataView(bytes.buffer).setBigUint64(4, counter);
	return bytes;
}

/** One direction of the record layer: AES-256-GCM, nonce = 4 zero bytes then the u64 BE counter. */
export class RecordCipher {
	private counter = 0n;

	constructor(private readonly key: CryptoKey) {}

	async seal(plaintext: Uint8Array): Promise<Uint8Array> {
		if (plaintext.length > MAX_RECORD_PLAINTEXT)
			throw new Error("A stream record is larger than 16 KiB");
		const iv = nonce(this.counter);
		this.counter += 1n;
		return new Uint8Array(
			await crypto.subtle.encrypt(
				{ name: "AES-GCM", iv: iv as BufferSource },
				this.key,
				plaintext as BufferSource,
			),
		);
	}

	async open(record: Uint8Array): Promise<Uint8Array> {
		if (record.length < TAG_BYTES || record.length > 16 * 1024)
			throw new Error("A stream record has an invalid size");
		const iv = nonce(this.counter);
		const plain = await crypto.subtle.decrypt(
			{ name: "AES-GCM", iv: iv as BufferSource },
			this.key,
			record as BufferSource,
		);
		this.counter += 1n;
		return new Uint8Array(plain);
	}
}

/**
 * The viewer's end of the record layer over a data channel stream: sends the empty key-confirmation
 * record first, then encrypts every write and decrypts every read. Any failure resets the stream.
 */
export class RecordStream implements ScreenStream {
	private readonly outgoing: RecordCipher;
	private readonly incoming: RecordCipher;
	private writes: Promise<unknown> = Promise.resolve();

	constructor(
		private readonly inner: ScreenStream,
		keys: StreamKeys,
	) {
		this.outgoing = new RecordCipher(keys.viewerToHost);
		this.incoming = new RecordCipher(keys.hostToViewer);
		this.enqueue(new Uint8Array(0)).catch(() => this.reset());
	}

	private enqueue(plaintext: Uint8Array): Promise<void> {
		const write = async () => {
			await this.inner.write(await this.outgoing.seal(plaintext));
		};
		const next = this.writes.then(write);
		this.writes = next.catch(() => {});
		return next;
	}

	async read(): Promise<Uint8Array | null> {
		for (;;) {
			const record = await this.inner.read();
			if (record === null) return null;
			try {
				const plain = await this.incoming.open(record);
				if (plain.length > 0) return plain;
			} catch {
				this.reset();
				return null;
			}
		}
	}

	async write(bytes: Uint8Array): Promise<void> {
		try {
			for (
				let offset = 0;
				offset < bytes.length;
				offset += MAX_RECORD_PLAINTEXT
			) {
				await this.enqueue(
					bytes.subarray(offset, offset + MAX_RECORD_PLAINTEXT),
				);
			}
		} catch (error) {
			this.reset();
			throw error;
		}
	}

	reset(): void {
		this.inner.reset();
	}
}
