"use client";

import {
	Check,
	ChevronLeft,
	ChevronRight,
	Loader2,
	Package,
	Plus,
	Search,
	X,
} from "lucide-react";
import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { getApiOrigin } from "../../lib/api-url";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import type { IBitSearchQuery } from "../../lib/schema/hub/bit-search-query";
import { useBackend } from "../../state/backend-state";
import type { IProfile } from "../../types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
	appendProfileBitReference,
	findProfileBit,
	profileBitDetails,
	profileBitReference,
	profileBitTypeLabel,
} from "./profile-bits-helpers";

const PAGE_SIZE = 24;
const MODEL_TYPES = [
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.Embedding,
	IBitTypes.ImageEmbedding,
	IBitTypes.Stt,
	IBitTypes.Tts,
	IBitTypes.ImageGeneration,
	IBitTypes.VideoGeneration,
	IBitTypes.ObjectDetection,
];

export interface ProfileBitsPickerProps {
	value: string[];
	onChange: (value: string[]) => void;
	disabled?: boolean;
}

export function ProfileBitsPicker({
	value,
	onChange,
	disabled = false,
}: ProfileBitsPickerProps) {
	const backend = useBackend();
	const auth = useAuth();
	const id = useId();
	const [search, setSearch] = useState("");
	const [debouncedSearch, setDebouncedSearch] = useState("");
	const [type, setType] = useState("models");
	const [page, setPage] = useState(0);
	const [customReference, setCustomReference] = useState("");
	const origin = getApiOrigin(backend.profile);
	const viewer = auth?.user?.profile?.sub ?? "";
	const scope = ["profile-template-bits", origin, backend.profile?.id, viewer];
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		true,
		scope,
	);
	const identity = JSON.stringify([
		...scope,
		profile.data?.id,
		getApiOrigin(profile.data),
	]);
	const [known, setKnown] = useState<{ identity: string; bits: IBit[] }>({
		identity: "",
		bits: [],
	});

	useEffect(() => {
		const timeout = setTimeout(() => setDebouncedSearch(search.trim()), 250);
		return () => clearTimeout(timeout);
	}, [search]);

	const query = useMemo<IBitSearchQuery>(
		() => ({
			search: debouncedSearch || undefined,
			bit_types:
				type === "all"
					? undefined
					: type === "models"
						? MODEL_TYPES
						: [type as IBitTypes],
			limit: PAGE_SIZE,
			offset: page * PAGE_SIZE,
		}),
		[debouncedSearch, type, page],
	);
	const searchCatalogue = useCallback(
		async (currentProfile: IProfile | undefined, request: IBitSearchQuery) => {
			if (!currentProfile) throw new Error("Your profile is unavailable.");
			return backend.apiState.post<IBit[]>(currentProfile, "bit", request);
		},
		[backend.apiState],
	);
	const catalogue = useInvoke(
		searchCatalogue,
		undefined,
		[profile.data, query],
		Boolean(profile.data),
		[identity],
	);

	useEffect(() => {
		if (!catalogue.data) return;
		setKnown((previous) => {
			const bits = new Map(
				(previous.identity === identity ? previous.bits : []).map((bit) => [
					profileBitReference(bit),
					bit,
				]),
			);
			for (const bit of catalogue.data) bits.set(profileBitReference(bit), bit);
			return { identity, bits: [...bits.values()] };
		});
	}, [catalogue.data, identity]);

	const knownBits = useMemo(() => {
		const bits = new Map(
			(known.identity === identity ? known.bits : []).map((bit) => [
				profileBitReference(bit),
				bit,
			]),
		);
		for (const bit of catalogue.data ?? [])
			bits.set(profileBitReference(bit), bit);
		return [...bits.values()];
	}, [known, identity, catalogue.data]);
	const results = useMemo(
		() => [
			...new Map(
				(catalogue.data ?? []).map((bit) => [profileBitReference(bit), bit]),
			).values(),
		],
		[catalogue.data],
	);
	const selected = new Set(value);
	const pendingSearch = search.trim() !== debouncedSearch;
	const loading = profile.isPending || catalogue.isPending || pendingSearch;
	const failure = profile.error || catalogue.error;
	const addReference = () => {
		if (
			disabled ||
			!customReference.trim() ||
			selected.has(customReference.trim())
		)
			return;
		onChange(appendProfileBitReference(value, customReference));
		setCustomReference("");
	};
	const removeReference = (reference: string) => {
		if (!disabled) onChange(value.filter((item) => item !== reference));
	};

	return (
		<div className="min-w-0 space-y-5">
			<section aria-labelledby={`${id}-selected`} className="space-y-3">
				<div className="flex items-center gap-2">
					<h3 id={`${id}-selected`} className="text-sm font-medium">
						Included bits
					</h3>
					<Badge variant="secondary">{selected.size}</Badge>
				</div>
				{value.length === 0 ? (
					<p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
						Choose models or other bits to include in this starter profile.
					</p>
				) : (
					<div className="grid gap-2 sm:grid-cols-2">
						{[...selected].map((reference) => {
							const bit = findProfileBit(reference, knownBits);
							const details = bit ? profileBitDetails(bit) : null;
							return (
								<div
									key={reference}
									className="flex min-w-0 items-start gap-3 rounded-lg border bg-muted/20 p-3"
								>
									<Package
										className="mt-0.5 size-4 shrink-0 text-muted-foreground"
										aria-hidden="true"
									/>
									<div className="min-w-0 flex-1">
										<p className="break-words text-sm font-medium">
											{details?.name ?? reference}
										</p>
										{bit && (
											<p className="mt-0.5 break-words text-xs text-muted-foreground">
												{details?.provider} · {profileBitTypeLabel(bit.type)}
											</p>
										)}
										<p className="mt-1 break-all text-xs text-muted-foreground">
											{bit
												? reference
												: "Saved reference. Kept even when it is outside the current catalogue."}
										</p>
									</div>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="size-7 shrink-0"
										disabled={disabled}
										aria-label={`Remove ${details?.name ?? reference}`}
										onClick={() => removeReference(reference)}
									>
										<X className="size-4" />
									</Button>
								</div>
							);
						})}
					</div>
				)}
			</section>

			<section
				aria-labelledby={`${id}-catalogue`}
				className="space-y-3 rounded-xl border p-4"
			>
				<h3 id={`${id}-catalogue`} className="text-sm font-medium">
					Browse the catalogue
				</h3>
				<div className="flex flex-col gap-3 sm:flex-row">
					<div className="min-w-0 flex-1 space-y-1.5">
						<Label htmlFor={`${id}-search`}>Search bits</Label>
						<div className="relative">
							<Search
								className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground"
								aria-hidden="true"
							/>
							<Input
								id={`${id}-search`}
								value={search}
								disabled={disabled}
								placeholder="Search names and descriptions…"
								onKeyDown={(event) => {
									if (event.key === "Enter") event.preventDefault();
								}}
								className="pl-9"
								onChange={(event) => {
									setSearch(event.target.value);
									setPage(0);
								}}
							/>
						</div>
					</div>
					<div className="space-y-1.5 sm:w-48">
						<Label htmlFor={`${id}-type`}>Type</Label>
						<select
							id={`${id}-type`}
							value={type}
							disabled={disabled}
							className="h-10 w-full min-w-0 rounded-md border bg-background px-3 text-sm disabled:opacity-50"
							onChange={(event) => {
								setType(event.target.value);
								setPage(0);
							}}
						>
							<option value="models">All model types</option>
							<option value="all">All bit types</option>
							{Object.values(IBitTypes).map((item) => (
								<option key={item} value={item}>
									{profileBitTypeLabel(item)}
								</option>
							))}
						</select>
					</div>
				</div>
				{failure ? (
					<div
						role="alert"
						className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm"
					>
						<p className="font-medium">The catalogue could not be loaded.</p>
						<p className="mt-1 break-words text-muted-foreground">
							{failure.message}
						</p>
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="mt-3"
							disabled={disabled || catalogue.isFetching || profile.isFetching}
							onClick={() =>
								void (profile.error ? profile.refetch() : catalogue.refetch())
							}
						>
							Try again
						</Button>
					</div>
				) : loading ? (
					<output className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
						<Loader2 className="size-4 animate-spin" aria-hidden="true" />
						Loading bits…
					</output>
				) : results.length === 0 ? (
					<output className="block py-8 text-center text-sm text-muted-foreground">
						{search || type !== "models"
							? "No bits match these filters."
							: "No models are available in this catalogue yet."}
					</output>
				) : (
					<div
						className="grid max-h-[28rem] gap-2 overflow-y-auto sm:grid-cols-2"
						aria-busy={catalogue.isFetching}
					>
						{results.map((bit) => {
							const reference = profileBitReference(bit);
							const details = profileBitDetails(bit);
							const included = selected.has(reference) || selected.has(bit.id);
							return (
								<button
									key={reference}
									type="button"
									disabled={disabled}
									aria-pressed={included}
									aria-label={`${included ? "Remove" : "Include"} ${details.name}`}
									className={`flex min-w-0 items-start gap-3 rounded-lg border p-3 text-left transition-colors hover:bg-muted/50 disabled:pointer-events-none disabled:opacity-50 ${included ? "border-primary/50 bg-primary/5" : "bg-background"}`}
									onClick={() => {
										if (included)
											onChange(
												value.filter(
													(item) => item !== reference && item !== bit.id,
												),
											);
										else onChange(appendProfileBitReference(value, reference));
									}}
								>
									<div className="min-w-0 flex-1">
										<p className="break-words text-sm font-medium">
											{details.name}
										</p>
										<p className="mt-1 break-words text-xs text-muted-foreground">
											{details.provider}
										</p>
										<Badge
											variant="outline"
											className="mt-2 whitespace-normal text-left text-[10px]"
										>
											{profileBitTypeLabel(bit.type)}
										</Badge>
										{details.description && (
											<p className="mt-2 line-clamp-2 break-words text-xs text-muted-foreground">
												{details.description}
											</p>
										)}
									</div>
									{included ? (
										<Check
											className="size-4 shrink-0 text-primary"
											aria-hidden="true"
										/>
									) : (
										<Plus
											className="size-4 shrink-0 text-muted-foreground"
											aria-hidden="true"
										/>
									)}
								</button>
							);
						})}
					</div>
				)}
				<div className="flex items-center justify-between gap-2 border-t pt-3 text-xs text-muted-foreground">
					<span aria-live="polite">
						Page {page + 1}
						{!loading && !failure ? ` · ${results.length} results` : ""}
					</span>
					<div className="flex gap-1">
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={disabled || page === 0 || loading}
							onClick={() => setPage((current) => Math.max(0, current - 1))}
						>
							<ChevronLeft className="size-4" />
							Previous
						</Button>
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={
								disabled ||
								loading ||
								Boolean(failure) ||
								(catalogue.data?.length ?? 0) < PAGE_SIZE
							}
							onClick={() => setPage((current) => current + 1)}
						>
							Next
							<ChevronRight className="size-4" />
						</Button>
					</div>
				</div>
			</section>

			<details className="rounded-lg border p-4">
				<summary className="cursor-pointer text-sm font-medium">
					Add a custom bit reference
				</summary>
				<p className="mt-2 text-xs text-muted-foreground">
					Use a known reference for a bit from another hub or one that is not
					listed here.
				</p>
				<div className="mt-3 space-y-1.5">
					<Label htmlFor={`${id}-reference`}>Bit reference</Label>
					<div className="flex flex-col gap-2 sm:flex-row">
						<Input
							id={`${id}-reference`}
							value={customReference}
							disabled={disabled}
							placeholder="hub.example.com:bit-id"
							className="min-w-0 flex-1"
							onChange={(event) => setCustomReference(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									addReference();
								}
							}}
						/>
						<Button
							type="button"
							variant="outline"
							disabled={
								disabled ||
								!customReference.trim() ||
								selected.has(customReference.trim())
							}
							onClick={addReference}
						>
							<Plus className="size-4" />
							Add reference
						</Button>
					</div>
					{customReference.trim() && selected.has(customReference.trim()) && (
						<output className="text-xs text-muted-foreground">
							This reference is already included.
						</output>
					)}
				</div>
			</details>
		</div>
	);
}
