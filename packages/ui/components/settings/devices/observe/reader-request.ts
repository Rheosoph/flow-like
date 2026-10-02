import type { ArchiveRecipient } from "../../../../lib/device-management/types";

/** The file a person sends the owner to be added as a reader of retained history. */
export const READER_REQUEST_KIND = "flow-like.history-reader-request";
const MAX_REQUESTS = 31;

export interface ReaderRequestFile {
	kind: typeof READER_REQUEST_KIND;
	version: 1;
	device_id: string;
	recipient: ArchiveRecipient;
}

export function readerRequestOf(
	deviceId: string,
	recipient: ArchiveRecipient,
): ReaderRequestFile {
	return {
		kind: READER_REQUEST_KIND,
		version: 1,
		device_id: deviceId,
		recipient,
	};
}

export type ReaderRequestProblem = "invalid" | "other_device" | "too_many";

export type ParsedReaderRequest =
	| { ok: true; recipients: ArchiveRecipient[] }
	| { ok: false; problem: ReaderRequestProblem };

const ID = /^[A-Za-z0-9_:.|@-]{1,256}$/u;

const isId = (value: unknown): value is string =>
	typeof value === "string" && ID.test(value);

const isByte = (value: unknown) =>
	Number.isInteger(value) && Number(value) >= 0 && Number(value) <= 255;

const isKey = (value: unknown): value is number[] =>
	Array.isArray(value) &&
	value.length >= 1 &&
	value.length <= 64 &&
	value.every(isByte);

function recipientOf(value: unknown): ArchiveRecipient | undefined {
	if (!value || typeof value !== "object") return undefined;
	const row = value as Partial<ArchiveRecipient>;
	if (!isId(row.recipient_id) || !isId(row.user_id) || !isKey(row.public_key))
		return undefined;
	return {
		recipient_id: row.recipient_id,
		user_id: row.user_id,
		public_key: [...row.public_key],
	};
}

function parseJson(text: string): { value: unknown } | undefined {
	try {
		return { value: JSON.parse(text) };
	} catch {
		return undefined;
	}
}

/** The readers a parsed value names: the request file's one reader, or what it is itself. */
function requested(
	value: unknown,
	deviceId: string,
): { rows: unknown[] } | { problem: ReaderRequestProblem } {
	const file = value as Partial<ReaderRequestFile> | null;
	const isFile =
		!!file && typeof file === "object" && file.kind === READER_REQUEST_KIND;
	if (isFile && file.device_id !== deviceId) return { problem: "other_device" };
	const named = isFile ? file.recipient : value;
	return { rows: Array.isArray(named) ? named : [named] };
}

/**
 * Reads a reader request: the file of `readerRequestOf`, one bare reader, or a
 * list of readers (what older versions of the app showed as text).
 */
export function parseReaderRequest(
	text: string,
	deviceId: string,
): ParsedReaderRequest {
	const parsed = parseJson(text);
	if (!parsed) return { ok: false, problem: "invalid" };
	const named = requested(parsed.value, deviceId);
	if ("problem" in named) return { ok: false, problem: named.problem };
	if (named.rows.length > MAX_REQUESTS)
		return { ok: false, problem: "too_many" };
	const recipients = named.rows.flatMap((row) => {
		const recipient = recipientOf(row);
		return recipient ? [recipient] : [];
	});
	return recipients.length && recipients.length === named.rows.length
		? { ok: true, recipients }
		: { ok: false, problem: "invalid" };
}
