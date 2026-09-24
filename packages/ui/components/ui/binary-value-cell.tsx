"use client";

import { useTranslation } from "@flow-like/locales";
import { Binary, Braces, Copy, FileText, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	BINARY_HEAD_BYTES,
	type BinaryDecodeOptions,
	type BinaryReading,
	CELL_DECODE_OPTIONS,
	DETAIL_DECODE_OPTIONS,
	binaryByteLength,
	binaryBytes,
	bytesToBase64,
	formatHexDump,
} from "../../lib/binary-value";
import {
	peekBinaryReading,
	readBinaryValue,
} from "../../lib/binary-value-worker-client";
import { cn, humanFileSize } from "../../lib/utils";
import { Badge } from "./badge";
import { Button } from "./button";

/** Payloads this large show a spinner while they decode; smaller ones settle within a frame. */
const LARGE_PAYLOAD_BYTES = 64 * 1024;

/** The detail view stops rendering here; copying still takes the whole value. */
const RENDER_TEXT_LIMIT = 200_000;

interface BinaryReadingState {
	/** Undefined while decoding; null when the value holds no bytes. */
	reading: BinaryReading | null | undefined;
	pending: boolean;
}

export function useBinaryReading(
	value: unknown,
	options: BinaryDecodeOptions,
): BinaryReadingState {
	const [settled, setSettled] = useState<{
		value: unknown;
		reading: BinaryReading | null;
	}>();

	useEffect(() => {
		if (peekBinaryReading(value, options)) return;
		let cancelled = false;
		readBinaryValue(value, options)
			.catch(() => null)
			.then((reading) => {
				if (!cancelled) setSettled({ value, reading });
			});
		return () => {
			cancelled = true;
		};
	}, [value, options]);

	const reading =
		peekBinaryReading(value, options) ??
		(settled && settled.value === value ? settled.reading : undefined);
	return { reading, pending: reading === undefined };
}

const FORMAT_LABELS: Record<string, string> = {
	gzip: "gzip",
	zlib: "zlib",
	zstd: "Zstandard",
	png: "PNG",
	jpeg: "JPEG",
	gif: "GIF",
	webp: "WebP",
	pdf: "PDF",
	zip: "ZIP",
	parquet: "Parquet",
};

function ReadingIcon({
	reading,
	pending,
	large,
}: Readonly<{
	reading: BinaryReading | null | undefined;
	pending: boolean;
	large: boolean;
}>) {
	const className = "h-3 w-3 shrink-0 text-muted-foreground";
	if (pending && large) {
		return <Loader2 className={cn(className, "animate-spin")} />;
	}
	if (reading?.kind === "json") return <Braces className={className} />;
	if (reading?.kind === "text") return <FileText className={className} />;
	return <Binary className={className} />;
}

function CompressionTag({
	reading,
}: Readonly<{ reading?: BinaryReading | null }>) {
	if (!reading?.compression) return null;
	return (
		<span className="shrink-0 rounded border px-1 text-[10px] leading-4 text-muted-foreground">
			{reading.compression === "deflate" ? "zlib" : reading.compression}
		</span>
	);
}

const oneLine = (text: string) => text.replace(/\s+/g, " ").trim();

/**
 * One row-height reading of a binary cell: the decoded JSON or text when the
 * bytes hold any, otherwise the size and, when recognizable, the format.
 */
export function BinaryCellPreview({
	value,
	onClick,
	className,
}: Readonly<{
	value: unknown;
	onClick?: () => void;
	className?: string;
}>) {
	const { t } = useTranslation("common");
	const { reading, pending } = useBinaryReading(value, CELL_DECODE_OPTIONS);
	const byteLength = binaryByteLength(value);
	const size = humanFileSize(byteLength);

	let label: string;
	if (reading?.kind === "json" || reading?.kind === "text") {
		label = oneLine(reading.text) || t("emptyText", "Empty text");
	} else if (reading?.kind === "binary" && reading.format) {
		label = `${FORMAT_LABELS[reading.format] ?? reading.format} · ${size}`;
	} else {
		label = `${t("binary", "Binary")} · ${size}`;
	}

	const content = (
		<>
			<ReadingIcon
				reading={reading}
				pending={pending}
				large={byteLength >= LARGE_PAYLOAD_BYTES}
			/>
			<CompressionTag reading={reading} />
			<span
				className={cn(
					"min-w-0 truncate",
					(reading?.kind === "json" || reading?.kind === "text") &&
						"font-mono text-xs",
				)}
			>
				{label}
			</span>
		</>
	);

	if (!onClick) {
		return (
			<span className={cn("flex min-w-0 items-center gap-1.5", className)}>
				{content}
			</span>
		);
	}
	return (
		<Button
			variant="ghost"
			size="sm"
			className={cn(
				"h-6 max-w-[240px] justify-start gap-1.5 px-2 font-normal",
				className,
			)}
			onClick={onClick}
		>
			{content}
		</Button>
	);
}

function copyToClipboard(text: string, message: string) {
	void navigator.clipboard.writeText(text).then(() => toast.success(message));
}

function HexDump({
	bytes,
	total,
}: Readonly<{ bytes: Uint8Array; total: number }>) {
	const { t } = useTranslation("common");
	return (
		<div className="space-y-1.5">
			<pre className="max-h-[50vh] overflow-auto rounded-md bg-muted p-3 font-mono text-xs leading-5">
				{formatHexDump(bytes)}
			</pre>
			{bytes.byteLength < total && (
				<p className="text-xs text-muted-foreground">
					{t("showingFirstBytesOfTotal", "First {{shown}} of {{total}} bytes", {
						shown: bytes.byteLength.toLocaleString(),
						total: total.toLocaleString(),
					})}
				</p>
			)}
		</div>
	);
}

function DecodedText({ text }: Readonly<{ text: string }>) {
	const { t } = useTranslation("common");
	const clipped = text.length > RENDER_TEXT_LIMIT;
	return (
		<div className="space-y-1.5">
			<pre className="max-h-[50vh] overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted p-3 font-mono text-xs">
				{clipped ? text.slice(0, RENDER_TEXT_LIMIT) : text}
			</pre>
			{clipped && (
				<p className="text-xs text-muted-foreground">
					{t(
						"showingFirstCharactersCopyForAll",
						"Showing the first {{shown}} of {{total}} characters. Copy the value to get all of it.",
						{
							shown: RENDER_TEXT_LIMIT.toLocaleString(),
							total: text.length.toLocaleString(),
						},
					)}
				</p>
			)}
		</div>
	);
}

function ReadingSummary({ reading }: Readonly<{ reading: BinaryReading }>) {
	const { t } = useTranslation("common");
	if (reading.kind === "json") {
		return reading.shape === "array"
			? t("jsonArrayItems", {
					count: reading.entries,
					defaultValue_one: "JSON array · {{count, number}} item",
					defaultValue_other: "JSON array · {{count, number}} items",
				})
			: t("jsonObjectKeys", {
					count: reading.entries,
					defaultValue_one: "JSON object · {{count, number}} key",
					defaultValue_other: "JSON object · {{count, number}} keys",
				});
	}
	if (reading.kind === "text") return t("text", "Text");
	const format = reading.format
		? (FORMAT_LABELS[reading.format] ?? reading.format)
		: null;
	return format ?? t("binary", "Binary");
}

/**
 * The roomy reading of a binary value. Decoding runs off the main thread and
 * only up to a size budget; past it, or when the bytes are not text, the value
 * shows as a hex dump of its first bytes.
 */
export function BinaryValueDetail({
	value,
	className,
}: Readonly<{ value: unknown; className?: string }>) {
	const { t } = useTranslation("common");
	const { reading, pending } = useBinaryReading(value, DETAIL_DECODE_OPTIONS);
	const [showBytes, setShowBytes] = useState(false);
	const byteLength = binaryByteLength(value);
	const storedHead = useMemo(
		() =>
			binaryBytes(
				Array.isArray(value) ? value.slice(0, BINARY_HEAD_BYTES) : value,
			) ?? new Uint8Array(),
		[value],
	);

	if (pending) {
		return (
			<div
				className={cn(
					"flex items-center gap-2 py-6 text-sm text-muted-foreground",
					className,
				)}
			>
				<Loader2 className="h-4 w-4 animate-spin" />
				{t("decodingSize", "Decoding {{size}}…", {
					size: humanFileSize(byteLength),
				})}
			</div>
		);
	}

	const decoded =
		reading?.kind === "json" || reading?.kind === "text" ? reading : null;
	const copyValue = () => {
		if (decoded && !showBytes) {
			copyToClipboard(decoded.text, t("copied", "Copied!"));
			return;
		}
		const bytes = binaryBytes(value);
		if (bytes) copyToClipboard(bytesToBase64(bytes), t("copied", "Copied!"));
	};

	return (
		<div className={cn("space-y-3", className)}>
			<div className="flex flex-wrap items-center gap-2">
				{reading && (
					<Badge variant="secondary">
						<ReadingSummary reading={reading} />
					</Badge>
				)}
				<span className="text-xs tabular-nums text-muted-foreground">
					{reading?.compression && decoded
						? `${reading.compression === "deflate" ? "zlib" : reading.compression} · ${humanFileSize(byteLength)} → ${humanFileSize(decoded.decodedLength)}`
						: humanFileSize(byteLength)}
				</span>
				<div className="ml-auto flex items-center gap-1">
					{decoded && (
						<div className="flex rounded-md border p-0.5">
							<Button
								variant={showBytes ? "ghost" : "secondary"}
								size="sm"
								className="h-6 px-2 text-xs"
								onClick={() => setShowBytes(false)}
							>
								{t("decoded", "Decoded")}
							</Button>
							<Button
								variant={showBytes ? "secondary" : "ghost"}
								size="sm"
								className="h-6 px-2 text-xs"
								onClick={() => setShowBytes(true)}
							>
								{t("rawBytes", "Raw bytes")}
							</Button>
						</div>
					)}
					<Button
						variant="ghost"
						size="sm"
						className="h-7 gap-1.5 px-2 text-xs"
						onClick={copyValue}
					>
						<Copy className="h-3.5 w-3.5" />
						{decoded && !showBytes
							? t("copy", "Copy")
							: t("copyAsBase64", "Copy as Base64")}
					</Button>
				</div>
			</div>

			{decoded && !showBytes ? (
				<DecodedText text={decoded.text} />
			) : (
				<>
					{reading?.kind === "binary" && (
						<p className="text-xs text-muted-foreground">
							{reading.reason === "too-large"
								? t(
										"binaryTooLargeToDecode",
										"Too large to decode here. Values up to {{limit}} are read as text or JSON.",
										{
											limit: humanFileSize(DETAIL_DECODE_OPTIONS.maxBytes),
										},
									)
								: reading.compression
									? t(
											"binaryInflatedNotText",
											"Decompressed, but the result is not text. Showing the decompressed bytes.",
										)
									: t(
											"binaryNotText",
											"These bytes are not text, so they are shown as a hex dump.",
										)}
						</p>
					)}
					{reading?.kind === "binary" ? (
						<HexDump bytes={reading.head} total={reading.payloadLength} />
					) : (
						<HexDump bytes={storedHead} total={byteLength} />
					)}
				</>
			)}
		</div>
	);
}
