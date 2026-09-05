"use client";

import { ImageIcon, ImageOff, Loader2, Upload, X } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	PROFILE_MEDIA_ACCEPT,
	type ProfileMediaKind,
	prepareProfileMedia,
	profileMediaUrl,
} from "./profile-media-image";

export interface ProfileMediaFieldProps {
	label: string;
	value?: string | null;
	onChange: (value: string | null) => void;
	kind: ProfileMediaKind;
	disabled?: boolean;
	upload: (file: Blob) => Promise<string>;
	onBusyChange?: (busy: boolean) => void;
}

export function ProfileMediaField({
	label,
	value,
	onChange,
	kind,
	disabled = false,
	upload,
	onBusyChange,
}: ProfileMediaFieldProps) {
	const id = useId();
	const input = useRef<HTMLInputElement>(null);
	const operation = useRef(0);
	const busyRef = useRef(false);
	const reportedBusy = useRef(false);
	const busyCallback = useRef(onBusyChange);
	busyCallback.current = onBusyChange;
	const reportBusy = useCallback((active: boolean) => {
		if (reportedBusy.current === active) return;
		reportedBusy.current = active;
		busyCallback.current?.(active);
	}, []);
	const [stage, setStage] = useState<"preparing" | "uploading" | null>(null);
	const [dragging, setDragging] = useState(false);
	const [draftUrl, setDraftUrl] = useState(value ?? "");
	const [preview, setPreview] = useState<string | null>(null);
	const [failedPreview, setFailedPreview] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [urlError, setUrlError] = useState(false);
	const [status, setStatus] = useState("");
	const busy = stage !== null;
	const locked = disabled || busy;
	const previewUrl = preview ?? value;
	const previewFailed = Boolean(previewUrl && failedPreview === previewUrl);
	const descriptionId = `${id}-hint${error ? ` ${id}-error` : ""}`;

	// biome-ignore lint/correctness/useExhaustiveDependencies: Changing artwork kind invalidates an upload even when the URL is unchanged.
	useEffect(() => {
		setDraftUrl(value ?? "");
		setError(null);
		setUrlError(false);
		operation.current++;
		busyRef.current = false;
		reportBusy(false);
		setStage(null);
		setPreview(null);
	}, [value, kind]);
	useEffect(
		() => () => {
			operation.current++;
			reportBusy(false);
		},
		[reportBusy],
	);
	useEffect(
		() => () => {
			if (preview) URL.revokeObjectURL(preview);
		},
		[preview],
	);

	const accept = async (files: FileList | File[]) => {
		if (disabled || busyRef.current || files.length === 0) return;
		setError(null);
		setUrlError(false);
		setStatus("");
		if (files.length !== 1) {
			setError("Choose one image at a time.");
			return;
		}
		const current = ++operation.current;
		busyRef.current = true;
		reportBusy(true);
		setStage("preparing");
		try {
			const prepared = await prepareProfileMedia(files[0], kind);
			if (current !== operation.current) return;
			setPreview(URL.createObjectURL(prepared));
			setStage("uploading");
			const url = await upload(prepared);
			if (current !== operation.current) return;
			if (!url.trim())
				throw new Error("The upload did not return an image URL. Try again.");
			onChange(url);
			setDraftUrl(url);
			setStatus("Image uploaded.");
		} catch (reason) {
			if (current === operation.current)
				setError(
					reason instanceof Error
						? reason.message
						: "The image could not be uploaded. Try again.",
				);
		} finally {
			if (current === operation.current) {
				busyRef.current = false;
				reportBusy(false);
				setStage(null);
				setPreview(null);
			}
		}
	};
	const applyUrl = () => {
		if (locked) return;
		try {
			const next = profileMediaUrl(draftUrl);
			setError(null);
			setUrlError(false);
			onChange(next);
			setDraftUrl(next ?? "");
			setStatus(next ? "Image URL applied." : "Image removed.");
		} catch (reason) {
			setUrlError(true);
			setError(
				reason instanceof Error ? reason.message : "Enter a valid image URL.",
			);
		}
	};

	return (
		<fieldset
			className="min-w-0 space-y-3"
			aria-busy={busy}
			disabled={disabled}
		>
			<legend className="mb-3 text-sm font-medium">{label}</legend>
			<input
				ref={input}
				type="file"
				accept={PROFILE_MEDIA_ACCEPT}
				className="sr-only"
				tabIndex={-1}
				aria-label={`Choose ${label} file`}
				aria-describedby={descriptionId}
				disabled={locked}
				onChange={(event) => {
					const files = Array.from(event.currentTarget.files ?? []);
					event.currentTarget.value = "";
					void accept(files);
				}}
			/>
			<button
				type="button"
				disabled={locked}
				aria-label={`Upload ${label}`}
				aria-describedby={descriptionId}
				onClick={() => input.current?.click()}
				onDragOver={(event) => {
					event.preventDefault();
					event.dataTransfer.dropEffect = locked ? "none" : "copy";
					if (!locked) setDragging(true);
				}}
				onDragLeave={() => setDragging(false)}
				onDrop={(event) => {
					event.preventDefault();
					setDragging(false);
					void accept(event.dataTransfer.files);
				}}
				className={cn(
					"group relative flex w-full min-w-0 flex-col items-center justify-center gap-2 overflow-hidden rounded-xl border border-dashed bg-muted/25 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
					kind === "cover" ? "aspect-[16/6] min-h-36" : "h-40",
					dragging
						? "border-primary bg-primary/10"
						: "border-border hover:border-primary/60 hover:bg-muted/50",
				)}
			>
				{previewUrl && !previewFailed && (
					<img
						src={previewUrl}
						alt={`${label} preview`}
						className={cn(
							"absolute inset-0 h-full w-full",
							kind === "icon" ? "object-contain p-3" : "object-cover",
						)}
						onError={() => setFailedPreview(previewUrl)}
					/>
				)}
				<div
					className={cn(
						"relative z-10 flex flex-col items-center gap-2 rounded-lg px-4 py-3 text-sm",
						previewUrl && !previewFailed
							? "bg-background/85 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
							: "text-muted-foreground",
						busy && "opacity-100",
					)}
				>
					{busy ? (
						<Loader2 className="size-5 animate-spin" />
					) : previewFailed ? (
						<ImageOff className="size-5" />
					) : previewUrl ? (
						<Upload className="size-5" />
					) : (
						<ImageIcon className="size-6" />
					)}
					<span>
						{stage === "preparing"
							? "Preparing image…"
							: stage === "uploading"
								? "Uploading…"
								: previewFailed
									? "Preview unavailable. Choose another image."
									: previewUrl
										? "Drop or choose a replacement"
										: "Drop an image here, or choose a file"}
					</span>
				</div>
			</button>
			<p
				id={`${id}-hint`}
				className="text-xs leading-relaxed text-muted-foreground"
			>
				PNG, JPEG, or WebP. Up to 10 MB and 4096 pixels per edge.
			</p>
			<div className="space-y-2">
				<Label htmlFor={`${id}-url`} className="text-xs">
					{label} URL
				</Label>
				<div className="flex min-w-0 flex-wrap gap-2">
					<Input
						id={`${id}-url`}
						type="text"
						inputMode="url"
						value={draftUrl}
						disabled={locked}
						className="min-w-40 flex-1"
						placeholder="https://example.com/image.webp"
						aria-invalid={urlError}
						aria-describedby={descriptionId}
						onChange={(event) => {
							setDraftUrl(event.target.value);
							if (urlError) {
								setError(null);
								setUrlError(false);
							}
						}}
						onKeyDown={(event) => {
							if (event.key === "Enter") {
								event.preventDefault();
								event.stopPropagation();
								applyUrl();
							}
						}}
					/>
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="h-9"
						disabled={locked || draftUrl.trim() === (value ?? "")}
						onClick={applyUrl}
					>
						Use URL
					</Button>
					{value && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-9"
							aria-label={`Remove ${label}`}
							disabled={locked}
							onClick={() => {
								onChange(null);
								setDraftUrl("");
								setFailedPreview(null);
								setError(null);
								setUrlError(false);
								setStatus("Image removed.");
							}}
						>
							<X className="size-4" />
							Remove
						</Button>
					)}
				</div>
			</div>
			{error && (
				<p
					id={`${id}-error`}
					role="alert"
					className="text-xs leading-relaxed text-destructive"
				>
					{error}
				</p>
			)}
			<output className="sr-only" aria-live="polite">
				{stage === "preparing"
					? "Preparing image."
					: stage === "uploading"
						? "Uploading image."
						: status}
			</output>
		</fieldset>
	);
}
