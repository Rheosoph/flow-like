"use client";

import { ImagePlus, Loader2, Plus, Upload, X } from "lucide-react";
import { type ReactNode, useId, useRef, useState } from "react";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Textarea } from "../ui/textarea";

export function EditorField({
	label,
	hint,
	value,
	onChange,
	multiline,
	type = "text",
	placeholder,
	disabled,
	required,
}: {
	label: string;
	hint?: string;
	value: string | number;
	onChange: (value: string) => void;
	multiline?: boolean;
	type?: string;
	placeholder?: string;
	disabled?: boolean;
	required?: boolean;
}) {
	const id = useId();
	return (
		<div className="space-y-2">
			<Label htmlFor={id}>
				{label}
				{required && <span className="text-muted-foreground"> *</span>}
			</Label>
			{multiline ? (
				<Textarea
					id={id}
					value={value}
					rows={4}
					onChange={(e) => onChange(e.target.value)}
					placeholder={placeholder}
					disabled={disabled}
					aria-describedby={hint ? `${id}-hint` : undefined}
				/>
			) : (
				<Input
					id={id}
					type={type}
					value={value}
					onChange={(e) => onChange(e.target.value)}
					placeholder={placeholder}
					disabled={disabled}
					aria-describedby={hint ? `${id}-hint` : undefined}
				/>
			)}
			{hint && (
				<p
					id={`${id}-hint`}
					className="text-xs leading-relaxed text-muted-foreground"
				>
					{hint}
				</p>
			)}
		</div>
	);
}
export function EditorSection({
	title,
	description,
	children,
}: { title: string; description?: string; children: ReactNode }) {
	return (
		<section className="space-y-5">
			<div>
				<h3 className="font-semibold tracking-tight">{title}</h3>
				{description && (
					<p className="mt-1 text-sm leading-relaxed text-muted-foreground">
						{description}
					</p>
				)}
			</div>
			{children}
		</section>
	);
}
export function StringList({
	label,
	value,
	onChange,
	placeholder = "Add an item",
	disabled,
}: {
	label: string;
	value: string[];
	onChange: (value: string[]) => void;
	placeholder?: string;
	disabled?: boolean;
}) {
	const [text, setText] = useState("");
	const id = useId();
	function add() {
		const items = text
			.split(/[,\n]/)
			.map((v) => v.trim())
			.filter(Boolean);
		if (items.length) onChange([...new Set([...value, ...items])]);
		setText("");
	}
	return (
		<div className="space-y-2">
			<Label htmlFor={id}>{label}</Label>
			<div className="rounded-lg border bg-background p-2.5 focus-within:ring-2 focus-within:ring-ring/30">
				<div className="flex flex-wrap gap-1.5">
					{value.map((item, index) => (
						<span
							key={`${index}:${item}`}
							className="inline-flex max-w-full items-center gap-1 rounded-md bg-muted py-1 pl-2.5 pr-1 text-xs"
						>
							<span className="break-all">{item}</span>
							<button
								type="button"
								disabled={disabled}
								onClick={() => onChange(value.filter((_, i) => i !== index))}
								aria-label={`Remove ${item}`}
								className="shrink-0 rounded p-1 hover:bg-background focus-visible:ring-2 focus-visible:ring-ring"
							>
								<X className="size-3" />
							</button>
						</span>
					))}
				</div>
				<div className="mt-1 flex gap-1">
					<Input
						id={id}
						disabled={disabled}
						value={text}
						onChange={(e) => setText(e.target.value)}
						onBlur={add}
						onKeyDown={(e) => {
							if (e.key === "Enter" || e.key === ",") {
								e.preventDefault();
								add();
							}
						}}
						placeholder={placeholder}
						className="h-8 border-0 bg-transparent px-1 shadow-none focus-visible:ring-0"
					/>
					<Button
						type="button"
						disabled={disabled || !text.trim()}
						variant="ghost"
						size="icon"
						className="size-8"
						aria-label={`Add to ${label.toLowerCase()}`}
						onClick={add}
					>
						<Plus className="size-4" />
					</Button>
				</div>
			</div>
		</div>
	);
}
export function BitImage({
	src,
	name,
	className = "size-12",
}: { src?: string | null; name: string; className?: string }) {
	const [failed, setFailed] = useState<string | null>(null);
	return (
		<div
			className={`flex shrink-0 items-center justify-center overflow-hidden rounded-xl bg-primary/8 text-primary ${className}`}
		>
			{src && failed !== src ? (
				<img
					src={src}
					alt={name}
					className="h-full w-full object-cover"
					onError={() => setFailed(src)}
				/>
			) : (
				<ImagePlus className="size-6 opacity-60" />
			)}
		</div>
	);
}

// Store a bounded thumbnail with the bit, keeping image selection local until Save.
export async function prepareBitImage(
	file: File,
	maxDimension: number,
): Promise<string> {
	if (
		!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)
	)
		throw new Error("Choose a PNG, JPEG, WebP, or GIF image.");
	if (file.size > 10 * 1024 * 1024)
		throw new Error("Choose an image smaller than 10 MB.");
	const objectUrl = URL.createObjectURL(file);
	try {
		const img = new Image();
		img.src = objectUrl;
		await img.decode();
		const ratio = Math.min(1, maxDimension / Math.max(img.width, img.height));
		const canvas = document.createElement("canvas");
		canvas.width = Math.max(1, Math.round(img.width * ratio));
		canvas.height = Math.max(1, Math.round(img.height * ratio));
		const context = canvas.getContext("2d");
		if (!context)
			throw new Error(
				"Image processing is unavailable. Use an image URL instead.",
			);
		context.drawImage(img, 0, 0, canvas.width, canvas.height);
		const data = canvas.toDataURL("image/webp", 0.82);
		if (data.length > 400_000)
			throw new Error("This image is too detailed. Try a smaller image.");
		return data;
	} finally {
		URL.revokeObjectURL(objectUrl);
	}
}
export function ImageField({
	label,
	value,
	onChange,
	wide,
	onBusyChange,
}: {
	label: string;
	value?: string | null;
	onChange: (value: string) => void;
	wide?: boolean;
	onBusyChange: (busy: boolean) => void;
}) {
	const input = useRef<HTMLInputElement>(null);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	async function select(file?: File) {
		if (!file || busy) return;
		setBusy(true);
		onBusyChange(true);
		setError(null);
		try {
			onChange(await prepareBitImage(file, wide ? 960 : 256));
		} catch (e) {
			setError(e instanceof Error ? e.message : "Could not read this image.");
		} finally {
			setBusy(false);
			onBusyChange(false);
			if (input.current) input.current.value = "";
		}
	}
	return (
		<div className="space-y-3">
			<p className="text-sm font-medium">{label}</p>
			<div
				className="rounded-xl border border-dashed bg-muted/20 p-4"
				onDragOver={(e) => e.preventDefault()}
				onDrop={(e) => {
					e.preventDefault();
					void select(e.dataTransfer.files[0]);
				}}
			>
				<div className={wide ? "space-y-4" : "flex items-center gap-4"}>
					<BitImage
						src={value}
						name={`${label} preview`}
						className={wide ? "aspect-[16/9] w-full" : "size-20"}
					/>
					<div className="space-y-2">
						<Button
							type="button"
							variant="outline"
							size="sm"
							disabled={busy}
							onClick={() => input.current?.click()}
						>
							{busy ? (
								<Loader2 className="size-4 animate-spin" />
							) : (
								<Upload className="size-4" />
							)}
							{value
								? `Replace ${label.toLowerCase()}`
								: `Upload ${label.toLowerCase()}`}
						</Button>
						<p className="text-xs text-muted-foreground">
							Or drop an image here. Up to 10 MB.
						</p>
					</div>
				</div>
				<input
					ref={input}
					aria-label={`Upload ${label.toLowerCase()} file`}
					type="file"
					accept="image/png,image/jpeg,image/webp,image/gif"
					className="sr-only"
					tabIndex={-1}
					disabled={busy}
					onChange={(e) => void select(e.target.files?.[0])}
				/>
			</div>
			<div className="flex items-end gap-2">
				<div className="min-w-0 flex-1">
					<EditorField
						label={`${label} URL`}
						value={value?.startsWith("data:") ? "" : (value ?? "")}
						placeholder={
							value?.startsWith("data:")
								? "Uploaded image selected"
								: "https://example.com/image.png"
						}
						onChange={onChange}
						disabled={busy}
					/>
				</div>
				{value && (
					<Button
						type="button"
						variant="ghost"
						size="icon"
						aria-label={`Remove ${label.toLowerCase()}`}
						disabled={busy}
						onClick={() => onChange("")}
					>
						<X className="size-4" />
					</Button>
				)}
			</div>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}
