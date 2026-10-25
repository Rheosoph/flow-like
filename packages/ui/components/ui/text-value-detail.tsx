"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy } from "lucide-react";
import { useMemo, useState } from "react";
import {
	type TextReading,
	parseJsonContainer,
	readTextValue,
} from "../../lib/text-value";
import { cn } from "../../lib/utils";
import { Badge } from "./badge";
import {
	ClippedText,
	JsonShapeSummary,
	ReadingToggle,
	copyToClipboard,
} from "./binary-value-cell";
import { Button } from "./button";
import { TextEditor } from "./text-editor";

/** JSON sources past this show pretty-printed instead of as a tree, which costs a few nodes per value. */
const JSON_TREE_LIMIT = 100_000;

function JsonString({ value }: Readonly<{ value: string }>) {
	const { t } = useTranslation("common");
	const nested = useMemo(() => parseJsonContainer(value), [value]);

	// Payloads often carry JSON serialized into a string field; read it as the
	// structure it is rather than as one escaped line.
	if (nested) {
		return (
			<>
				<span className="mr-1.5 rounded border px-1 font-sans text-[10px] leading-4 text-muted-foreground">
					{t("jsonString", "JSON string")}
				</span>
				<JsonNode value={nested} />
			</>
		);
	}
	if (value.includes("\n")) {
		return (
			<div className="my-0.5 whitespace-pre-wrap border-l-2 border-primary/40 pl-2">
				{value}
			</div>
		);
	}
	return <span>{JSON.stringify(value)}</span>;
}

function JsonNode({ value }: Readonly<{ value: unknown }>) {
	if (typeof value === "string") return <JsonString value={value} />;
	if (value === null || typeof value !== "object") {
		return <span className="text-primary">{String(value)}</span>;
	}

	const isArray = Array.isArray(value);
	const entries: [string | number, unknown][] = isArray
		? value.map((item, index) => [index, item])
		: Object.entries(value);
	const [open, close] = isArray ? ["[", "]"] : ["{", "}"];
	if (entries.length === 0) return <span>{`${open}${close}`}</span>;

	return (
		<>
			{open}
			<div className="ml-1 border-l border-border/60 pl-4">
				{entries.map(([key, item]) => (
					<div key={key}>
						<span className="text-muted-foreground">{`${key}: `}</span>
						<JsonNode value={item} />
					</div>
				))}
			</div>
			{close}
		</>
	);
}

function JsonValue({
	value,
	sourceLength,
}: Readonly<{ value: unknown; sourceLength: number }>) {
	const pretty = useMemo(
		() =>
			sourceLength > JSON_TREE_LIMIT ? JSON.stringify(value, null, 2) : null,
		[sourceLength, value],
	);
	if (pretty !== null) return <ClippedText text={pretty} />;
	return (
		<div className="rounded-md bg-muted p-3 font-mono text-xs leading-5 wrap-anywhere">
			<JsonNode value={value} />
		</div>
	);
}

function TextReadingSummary({ reading }: Readonly<{ reading: TextReading }>) {
	const { t } = useTranslation("common");
	if (reading.kind === "json") {
		const { value } = reading;
		return Array.isArray(value) ? (
			<JsonShapeSummary shape="array" entries={value.length} />
		) : (
			<JsonShapeSummary shape="object" entries={Object.keys(value).length} />
		);
	}
	if (reading.kind === "markup") return t("markup", "Markup");
	return t("text", "Text");
}

/**
 * The roomy reading of a stored string: JSON as a tree, XML and HTML verbatim,
 * prose as markdown. The raw string is one toggle away for every reading that
 * reshapes it.
 */
export function TextValueDetail({
	text,
	className,
}: Readonly<{ text: string; className?: string }>) {
	const { t } = useTranslation("common");
	const reading = useMemo(() => readTextValue(text), [text]);
	const [showRaw, setShowRaw] = useState(false);
	const reshaped = reading.kind !== "markup";

	return (
		<div className={cn("min-w-0 space-y-3", className)}>
			<div className="flex flex-wrap items-center gap-2">
				<Badge variant="secondary">
					<TextReadingSummary reading={reading} />
				</Badge>
				<span className="text-xs tabular-nums text-muted-foreground">
					{t("characterCount", {
						count: text.length,
						defaultValue_one: "{{count, number}} character",
						defaultValue_other: "{{count, number}} characters",
					})}
				</span>
				<div className="ml-auto flex items-center gap-1">
					{reshaped && (
						<ReadingToggle
							alternate={showRaw}
							onAlternateChange={setShowRaw}
							primaryLabel={t("formatted", "Formatted")}
							alternateLabel={t("raw", "Raw")}
						/>
					)}
					<Button
						variant="ghost"
						size="sm"
						className="h-7 gap-1.5 px-2 text-xs"
						onClick={() => copyToClipboard(text, t("copied", "Copied!"))}
					>
						<Copy className="h-3.5 w-3.5" />
						{t("copy", "Copy")}
					</Button>
				</div>
			</div>

			{!reshaped || showRaw ? (
				<ClippedText text={text} />
			) : reading.kind === "json" ? (
				<JsonValue value={reading.value} sourceLength={text.length} />
			) : (
				<TextEditor initialContent={text} isMarkdown={true} />
			)}
		</div>
	);
}
