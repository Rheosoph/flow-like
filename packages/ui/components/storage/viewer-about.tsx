"use client";

import { InfoIcon } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { isTauri } from "../../lib/platform";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export function ViewerAbout({
	label,
	licensesUrl,
	children,
}: {
	label: string;
	licensesUrl: string;
	children?: ReactNode;
}) {
	const storage = useBackend().storageState;
	const latestStorage = useRef(storage);
	latestStorage.current = storage;
	const frame = useRef<HTMLIFrameElement>(null);
	const detach = useRef<() => void>(() => {});
	const pending = useRef<AbortController | null>(null);
	const [open, setOpen] = useState(false);
	const [downloading, setDownloading] = useState(false);
	const [error, setError] = useState("");

	useEffect(() => {
		if (!open) {
			detach.current();
			pending.current?.abort();
			pending.current = null;
			setDownloading(false);
			setError("");
		}
		return () => {
			detach.current();
			pending.current?.abort();
		};
	}, [open]);

	function attachDownloads() {
		detach.current();
		if (!isTauri()) return;
		const document = frame.current?.contentDocument;
		if (!document) return;
		const download = (event: MouseEvent) => {
			const link = (event.target as Element | null)?.closest?.("a[download]");
			if (!link) return;
			event.preventDefault();
			if (pending.current) return;
			const controller = new AbortController();
			pending.current = controller;
			setDownloading(true);
			setError("");
			void saveSourceArchive(
				licensesUrl,
				(link as HTMLAnchorElement).href,
				controller.signal,
				async (filename, url) => {
					const storage = latestStorage.current;
					if (!storage.writeStorageItems)
						throw new Error("The native file save service is unavailable.");
					await storage.writeStorageItems([{ prefix: filename, url }]);
				},
			)
				.catch((cause) => {
					if (!controller.signal.aborted)
						setError(
							`${cause instanceof Error ? cause.message : "The source download failed."} Close and reopen About, then try again.`,
						);
				})
				.finally(() => {
					if (pending.current === controller) {
						pending.current = null;
						setDownloading(false);
					}
				});
		};
		document.addEventListener("click", download);
		detach.current = () => document.removeEventListener("click", download);
	}

	return (
		<Dialog open={open} onOpenChange={setOpen}>
			<Tooltip>
				<TooltipTrigger asChild>
					<DialogTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="size-8 shrink-0 text-muted-foreground"
							aria-label={`About ${label}`}
						>
							<InfoIcon />
						</Button>
					</DialogTrigger>
				</TooltipTrigger>
				<TooltipContent>Third-party licenses and source</TooltipContent>
			</Tooltip>
			<DialogContent className="h-[80dvh] overflow-hidden sm:max-w-4xl">
				<DialogHeader className="pr-6">
					<DialogTitle>About {label}</DialogTitle>
					<DialogDescription>
						{children ?? "Third-party licenses, notices, and source code."}
					</DialogDescription>
				</DialogHeader>
				{downloading && <output>Preparing source download...</output>}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<iframe
					ref={frame}
					src={licensesUrl}
					onLoad={attachDownloads}
					title={`${label} third-party licenses and source`}
					className="min-h-0 w-full flex-1 rounded-md border bg-white"
				/>
			</DialogContent>
		</Dialog>
	);
}

async function saveSourceArchive(
	licensesUrl: string,
	archiveUrl: string,
	signal: AbortSignal,
	save: (filename: string, url: string) => Promise<void>,
) {
	const index = new URL(licensesUrl, window.location.href);
	const archive = new URL(archiveUrl, index);
	const local = new URL(window.location.href);
	if (
		index.protocol !== local.protocol ||
		index.host !== local.host ||
		archive.protocol !== local.protocol ||
		archive.host !== local.host ||
		archive.username ||
		archive.password ||
		archive.search ||
		archive.hash ||
		!archive.pathname.startsWith(new URL("sources/", index).pathname)
	)
		throw new Error("This source download does not belong to the editor.");
	const response = await fetch(new URL("manifest.json", index), {
		signal,
		cache: "no-store",
		redirect: "error",
	});
	if (!response.ok)
		throw new Error("The source inventory could not be loaded.");
	const manifest = await response.json();
	const download = Array.isArray(manifest?.packages)
		? manifest.packages
				.map((entry: unknown) => sourceDownload(entry, index, archive))
				.find((entry: SourceDownload | undefined) => entry)
		: undefined;
	if (!download)
		throw new Error(
			"This source download is missing from the editor's inventory.",
		);
	if (signal.aborted) return;
	const source = await fetch(archive, {
		signal,
		cache: "no-store",
		redirect: "error",
	});
	if (!source.ok) throw new Error("The source archive could not be loaded.");
	const bytes = await source.arrayBuffer();
	if (
		download.byteLength !== undefined &&
		bytes.byteLength !== download.byteLength
	)
		throw new Error("The source archive failed its size check.");
	const checksum = Array.from(
		new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
		(value) => value.toString(16).padStart(2, "0"),
	).join("");
	if (checksum !== download.sha256)
		throw new Error("The source archive failed its integrity check.");
	if (signal.aborted) return;
	const url = URL.createObjectURL(
		new Blob([bytes], { type: "application/octet-stream" }),
	);
	try {
		await save(
			archive.pathname.slice(archive.pathname.lastIndexOf("/") + 1),
			url,
		);
	} finally {
		URL.revokeObjectURL(url);
	}
}

type SourceDownload = { sha256: string; byteLength?: number };

function sourceDownload(
	entry: unknown,
	index: URL,
	archive: URL,
): SourceDownload | undefined {
	if (!entry || typeof entry !== "object") return;
	const pkg = entry as Record<string, unknown>;
	if (
		typeof pkg.sourceArchive !== "string" ||
		!/^sources\/[\w.+-]+\.(?:crate|tar\.gz)$/.test(pkg.sourceArchive) ||
		typeof pkg.selectedLicense !== "string" ||
		!pkg.selectedLicense.split(" AND ").includes("MPL-2.0")
	)
		return;
	const validHash = (value: unknown): value is string =>
		typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
	const validLength = (value: unknown): value is number =>
		typeof value === "number" && Number.isSafeInteger(value) && value > 0;
	if (pkg.sourceArchiveParts !== undefined) {
		if (
			!Array.isArray(pkg.sourceArchiveParts) ||
			!validHash(pkg.sourceArchiveSha256) ||
			!validLength(pkg.sourceArchiveByteLength)
		)
			return;
		for (const part of pkg.sourceArchiveParts) {
			if (
				part &&
				typeof part === "object" &&
				typeof part.path === "string" &&
				part.path.startsWith(`${pkg.sourceArchive}.part`) &&
				/^\d{3,}$/.test(part.path.slice(pkg.sourceArchive.length + 5)) &&
				new URL(part.path, index).href === archive.href &&
				validHash(part.sha256) &&
				validLength(part.byteLength) &&
				part.byteLength <= pkg.sourceArchiveByteLength
			)
				return { sha256: part.sha256, byteLength: part.byteLength };
		}
		return;
	}
	if (new URL(pkg.sourceArchive, index).href !== archive.href) return;
	if (
		validHash(pkg.sourceArchiveSha256) &&
		validLength(pkg.sourceArchiveByteLength)
	)
		return {
			sha256: pkg.sourceArchiveSha256,
			byteLength: pkg.sourceArchiveByteLength,
		};
	if (
		pkg.sourceArchiveSha256 === undefined &&
		pkg.sourceArchiveByteLength === undefined &&
		pkg.sourceArchive.endsWith(".crate") &&
		validHash(pkg.checksum)
	)
		return { sha256: pkg.checksum };
}
