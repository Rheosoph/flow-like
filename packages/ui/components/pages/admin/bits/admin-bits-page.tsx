"use client";

import { useQuery } from "@tanstack/react-query";
import {
	ChevronLeft,
	ChevronRight,
	Pencil,
	Plus,
	RefreshCw,
	Search,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { useBackend } from "../../../../state/backend-state";
import { BitEditorDialog } from "../../../bits/bit-editor-dialog";
import { BitImage } from "../../../bits/bit-editor-fields";
import { bitMetadata, record } from "../../../bits/bit-editor-model";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Skeleton } from "../../../ui/skeleton";

export function AdminBitsPage() {
	const backend = useBackend();
	const router = useRouter();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const [search, setSearch] = useState("");
	const [debounced, setDebounced] = useState("");
	const [type, setType] = useState("all");
	const [page, setPage] = useState(1);
	const [limit, setLimit] = useState(24);
	const [selected, setSelected] = useState<IBit | null>(null);
	useEffect(() => {
		const timer = setTimeout(() => {
			setDebounced(search.trim());
			setPage(1);
		}, 250);
		return () => clearTimeout(timer);
	}, [search]);
	const bits = useQuery({
		queryKey: [
			"bit-search",
			"admin-editor",
			profile.data?.id,
			debounced,
			type,
			page,
			limit,
		],
		enabled: !!profile.data,
		queryFn: () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<IBit[]>(profile.data, "bit", {
				search: debounced || undefined,
				bit_types:
					type === "all"
						? undefined
						: type === "hosted"
							? [IBitTypes.Llm, IBitTypes.Vlm, IBitTypes.Tts, IBitTypes.Stt]
							: [type],
				limit,
				offset: (page - 1) * limit,
			});
		},
	});
	const detail = useQuery({
		queryKey: ["bit", selected?.id],
		enabled: !!selected && !!profile.data,
		queryFn: () => {
			if (!profile.data || !selected) throw new Error("Bit not selected");
			return backend.apiState.get<IBit>(
				profile.data,
				`bit/${encodeURIComponent(selected.id)}`,
			);
		},
		staleTime: 0,
	});
	const visible = (bits.data ?? []).filter(
		(bit) =>
			type !== "hosted" ||
			/^(hosted(?::|$)|premium$|internal$)/i.test(
				String(record(record(bit.parameters).provider).provider_name ?? ""),
			),
	);
	const failure = profile.error || bits.error;
	return (
		<main className="flex h-full min-h-0 w-full flex-1 flex-col overflow-y-auto bg-background">
			<div className="mx-auto w-full max-w-7xl space-y-7 p-5 sm:p-8">
				<header className="flex flex-wrap items-start justify-between gap-4">
					<div>
						<p className="mb-2 text-xs font-medium uppercase tracking-widest text-muted-foreground">
							Registry
						</p>
						<h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">
							Bits
						</h1>
						<p className="mt-2 text-sm text-muted-foreground">
							Manage the models and assets people use in their flows.
						</p>
					</div>
					<div className="flex gap-2">
						<Button
							variant="outline"
							size="sm"
							onClick={() => void bits.refetch()}
							disabled={bits.isFetching}
						>
							<RefreshCw
								className={`size-4 ${bits.isFetching ? "animate-spin" : ""}`}
							/>
							Refresh
						</Button>
						<Button
							className="bg-foreground text-background hover:bg-foreground/90"
							size="sm"
							onClick={() => router.push("/admin/bits/add")}
						>
							<Plus className="size-4" />
							Add bit
						</Button>
					</div>
				</header>
				<div className="flex flex-wrap gap-3">
					<div className="relative min-w-48 flex-1">
						<Search className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
						<Input
							aria-label="Search bits"
							placeholder="Search by name, description, or tag…"
							value={search}
							onChange={(e) => setSearch(e.target.value)}
							className="h-10 pl-10"
						/>
					</div>
					<select
						aria-label="Filter by bit type"
						value={type}
						onChange={(e) => {
							setType(e.target.value);
							setPage(1);
						}}
						className="h-10 rounded-lg border bg-background px-3 text-sm"
					>
						<option value="all">All types</option>
						<option value="hosted">Hosted models</option>
						{Object.values(IBitTypes).map((value) => (
							<option key={value}>{value}</option>
						))}
					</select>
				</div>
				<section
					aria-label="Registry bits"
					className="overflow-hidden rounded-xl border"
				>
					<div className="flex items-center justify-between border-b bg-muted/30 px-5 py-3">
						<h2 className="text-sm font-medium">
							{bits.isPending
								? "Loading bits…"
								: `${visible.length} bits on this page`}
						</h2>
						<p className="hidden text-xs text-muted-foreground sm:block">
							Select a bit to edit
						</p>
					</div>
					{failure ? (
						<div role="alert" className="space-y-3 p-8 text-center">
							<p className="text-sm text-destructive">
								Could not load bits. {failure.message}
							</p>
							<Button
								variant="outline"
								onClick={() => {
									void profile.refetch();
									void bits.refetch();
								}}
							>
								Try again
							</Button>
						</div>
					) : bits.isPending ? (
						<div className="space-y-3 p-5">
							{[1, 2, 3, 4].map((i) => (
								<Skeleton key={i} className="h-20 w-full" />
							))}
						</div>
					) : visible.length ? (
						<div className="divide-y">
							{visible.map((bit) => {
								const meta = bitMetadata(bit);
								return (
									<button
										type="button"
										key={bit.id}
										onClick={() => setSelected(bit)}
										className="group flex w-full items-center gap-4 px-5 py-4 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
									>
										<BitImage src={meta.icon} name="" />
										<div className="min-w-0 flex-1">
											<div className="flex flex-wrap items-center gap-2">
												<span className="truncate text-sm font-semibold">
													{meta.name || "Untitled bit"}
												</span>
												<Badge variant="secondary" className="text-[10px]">
													{bit.type}
												</Badge>
											</div>
											<p className="mt-1 truncate text-xs text-muted-foreground">
												{meta.description || bit.id}
											</p>
										</div>
										<span className="hidden max-w-40 truncate text-xs text-muted-foreground lg:block">
											{String(
												record(record(bit.parameters).provider).provider_name ||
													bit.version ||
													"",
											)}
										</span>
										<span className="flex items-center gap-2 text-xs font-medium text-muted-foreground group-hover:text-primary">
											<Pencil className="size-3.5" />
											<span className="hidden sm:inline">Edit</span>
										</span>
									</button>
								);
							})}
						</div>
					) : (
						<div className="space-y-2 p-12 text-center">
							<Search className="mx-auto mb-4 size-7 text-muted-foreground/50" />
							<h3 className="font-medium">No bits found</h3>
							<p className="text-sm text-muted-foreground">
								Try a different search or type. You can also check the next
								page.
							</p>
						</div>
					)}
				</section>
				<footer className="flex flex-wrap items-center justify-between gap-4">
					<label className="flex items-center gap-2 text-xs text-muted-foreground">
						Bits per page
						<select
							aria-label="Bits per page"
							value={limit}
							onChange={(e) => {
								setLimit(Number(e.target.value));
								setPage(1);
							}}
							className="rounded-md border bg-background p-2 text-foreground"
						>
							{[12, 24, 48, 96].map((size) => (
								<option key={size}>{size}</option>
							))}
						</select>
					</label>
					<div className="flex items-center gap-3">
						<Button
							size="icon"
							variant="outline"
							aria-label="Previous page"
							disabled={page === 1 || bits.isFetching}
							onClick={() => setPage((value) => value - 1)}
						>
							<ChevronLeft className="size-4" />
						</Button>
						<span className="text-xs text-muted-foreground">Page {page}</span>
						<Button
							size="icon"
							variant="outline"
							aria-label="Next page"
							disabled={(bits.data?.length ?? 0) < limit || bits.isFetching}
							onClick={() => setPage((value) => value + 1)}
						>
							<ChevronRight className="size-4" />
						</Button>
					</div>
				</footer>
				{selected && detail.isPending && (
					<output className="rounded-lg border p-4 text-sm text-muted-foreground">
						Loading bit editor…
					</output>
				)}
				{selected && detail.error && (
					<div
						role="alert"
						className="flex items-center gap-3 rounded-lg border p-4 text-sm text-destructive"
					>
						Could not load this bit.
						<Button
							size="sm"
							variant="outline"
							onClick={() => void detail.refetch()}
						>
							Try again
						</Button>
						<Button size="sm" variant="ghost" onClick={() => setSelected(null)}>
							Dismiss
						</Button>
					</div>
				)}
			</div>
			{selected && detail.data && (
				<BitEditorDialog
					bit={detail.data}
					open
					scope="admin"
					onOpenChange={(open) => {
						if (!open) setSelected(null);
					}}
					onSaved={(saved) => {
						if (saved.id !== selected.id) setSelected(saved);
					}}
					onDeleted={() => setSelected(null)}
				/>
			)}
		</main>
	);
}
