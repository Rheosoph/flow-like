"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	ArrowUpRight,
	Copy,
	LayoutTemplate,
	Loader2,
	Pencil,
	Plus,
	RefreshCw,
	Search,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import type { IProfile } from "../../lib/schema/profile/profile";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { filterProfileTemplates } from "./profile-template-model";
import { ProfileTemplatePreview } from "./profile-template-preview";
import { useProfileTemplates } from "./use-profile-templates";

export function ProfileTemplatesPage() {
	const context = useProfileTemplates();
	return <ProfileTemplatesContent key={context.scopeKey} context={context} />;
}

function ProfileTemplatesContent({
	context,
}: { context: ReturnType<typeof useProfileTemplates> }) {
	const client = useQueryClient();
	const [search, setSearch] = useState("");
	const [sort, setSort] = useState("updated");
	const [deleting, setDeleting] = useState<IProfile | null>(null);
	const [busy, setBusy] = useState(false);
	const [deleteError, setDeleteError] = useState<string | null>(null);
	const profiles = filterProfileTemplates(
		context.templates.data ?? [],
		search,
		sort,
	);
	const remove = async () => {
		if (!deleting?.id || !context.profile.data || !context.canWrite || busy)
			return;
		setBusy(true);
		setDeleteError(null);
		try {
			await context.backend.apiState.del(
				context.profile.data,
				`admin/profiles/${encodeURIComponent(deleting.id)}`,
			);
			client.setQueryData<IProfile[]>(context.queryKey, (current) =>
				current?.filter((item) => item.id !== deleting.id),
			);
			setDeleting(null);
			toast.success("Starter profile deleted");
			await Promise.all([
				client.invalidateQueries({ queryKey: ["profile-templates"] }),
				client.invalidateQueries({ queryKey: ["home-default-templates"] }),
				client.invalidateQueries({ queryKey: ["info", "profiles"] }),
			]);
		} catch (error) {
			setDeleteError(
				error instanceof Error
					? error.message
					: "The profile could not be deleted. Try again.",
			);
		} finally {
			setBusy(false);
		}
	};
	return (
		<main className="mx-auto w-full max-w-7xl space-y-7 px-4 py-6 sm:px-8 sm:py-8">
			<Link
				href="/admin"
				className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
			>
				<ArrowLeft className="h-4 w-4" />
				Administration
			</Link>
			<header className="flex flex-wrap items-end justify-between gap-4">
				<div className="max-w-2xl space-y-2">
					<p className="text-xs font-medium uppercase tracking-[0.18em] text-primary">
						Profile templates
					</p>
					<h1 className="text-3xl font-semibold tracking-tight sm:text-4xl">
						Starter profiles
					</h1>
					<p className="text-sm leading-relaxed text-muted-foreground">
						Shape the profiles people choose when they get started. Bring
						together a clear introduction, the right bits and apps, and a home
						that fits their work.
					</p>
				</div>
				{context.canWrite && (
					<Button asChild>
						<Link href="/admin/profiles/add">
							<Plus className="mr-2 h-4 w-4" />
							Create profile
						</Link>
					</Button>
				)}
			</header>
			{context.loading ? (
				<ProfileTemplateStatus message="Loading starter profiles…" />
			) : context.info.isError ||
				context.profile.isError ||
				context.templates.isError ? (
				<ProfileTemplateStatus
					message="Starter profiles could not be loaded."
					retry={() => {
						void context.info.refetch();
						void context.profile.refetch();
						void context.templates.refetch();
					}}
				/>
			) : !context.canRead ? (
				<ProfileTemplateStatus message="You need permission to view profile templates." />
			) : (
				<>
					<div className="flex flex-wrap items-center gap-3">
						<div className="relative min-w-0 flex-1 basis-60">
							<Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
							<Input
								aria-label="Search profiles"
								placeholder="Search by name, description, or tag…"
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								className="pl-9"
							/>
						</div>
						<select
							aria-label="Sort profiles"
							value={sort}
							onChange={(event) => setSort(event.target.value)}
							className="h-10 rounded-md border bg-background px-3 text-sm"
						>
							<option value="updated">Recently updated</option>
							<option value="name">Name A–Z</option>
						</select>
						<Button
							variant="outline"
							size="icon"
							aria-label="Refresh profiles"
							disabled={context.templates.isFetching}
							onClick={() => void context.templates.refetch()}
						>
							<RefreshCw
								className={`h-4 w-4 ${context.templates.isFetching ? "animate-spin" : ""}`}
							/>
						</Button>
					</div>
					<output className="block text-xs text-muted-foreground">
						{profiles.length} {profiles.length === 1 ? "profile" : "profiles"}
						{search && " matching your search"}
					</output>
					{context.templates.isLoading ? (
						<ProfileTemplateStatus message="Loading profiles…" />
					) : !profiles.length ? (
						<div className="flex flex-col items-center rounded-2xl border border-dashed p-10 text-center">
							<LayoutTemplate className="mb-4 h-9 w-9 text-primary/60" />
							<h2 className="text-lg font-semibold">
								{search
									? "No matching profiles"
									: "Give people a place to start"}
							</h2>
							<p className="mb-5 mt-2 max-w-md text-sm text-muted-foreground">
								{search
									? "Try another name or clear your search."
									: "Create a profile for a team, a role, or a way of working."}
							</p>
							{search ? (
								<Button variant="outline" onClick={() => setSearch("")}>
									Clear search
								</Button>
							) : (
								context.canWrite && (
									<Button asChild>
										<Link href="/admin/profiles/add">
											Create your first profile
										</Link>
									</Button>
								)
							)}
						</div>
					) : (
						<div className="grid items-stretch gap-5 md:grid-cols-2 xl:grid-cols-3">
							{profiles.map((profile) => (
								<article
									key={profile.id}
									className="group flex min-w-0 flex-col gap-3"
								>
									<ProfileTemplatePreview profile={profile} compact />
									<div className="flex flex-wrap items-center gap-1 px-1">
										{context.canWrite && (
											<>
												<Button variant="secondary" size="sm" asChild>
													<Link
														href={`/admin/profiles/add?id=${encodeURIComponent(profile.id ?? "")}`}
													>
														<Pencil className="mr-2 h-3.5 w-3.5" />
														Edit profile
													</Link>
												</Button>
												<Button
													size="icon"
													variant="ghost"
													aria-label={`Duplicate ${profile.name}`}
													asChild
												>
													<Link
														href={`/admin/profiles/add?copy=${encodeURIComponent(profile.id ?? "")}`}
													>
														<Copy className="h-4 w-4" />
													</Link>
												</Button>
											</>
										)}
										{context.canEditHome && (
											<Button variant="ghost" size="sm" asChild>
												<Link
													href={`/admin/home?default=${encodeURIComponent(profile.id ?? "")}`}
												>
													Home
													<ArrowUpRight className="ml-1 h-3.5 w-3.5" />
												</Link>
											</Button>
										)}
										{context.canWrite && (
											<Button
												variant="ghost"
												size="icon"
												className="ml-auto text-muted-foreground hover:text-destructive"
												aria-label={`Delete ${profile.name}`}
												onClick={() => {
													setDeleting(profile);
													setDeleteError(null);
												}}
											>
												<Trash2 className="h-4 w-4" />
											</Button>
										)}
									</div>
								</article>
							))}
						</div>
					)}
				</>
			)}
			<AlertDialog
				open={!!deleting}
				onOpenChange={(open) => {
					if (!open && !busy) setDeleting(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Delete {deleting?.name}?</AlertDialogTitle>
						<AlertDialogDescription>
							This removes it from the starter profile choices. Existing user
							profiles are kept. If it has a published home default, reset that
							home to follow the main default before deleting.
						</AlertDialogDescription>
					</AlertDialogHeader>
					{deleteError && (
						<p role="alert" className="break-words text-sm text-destructive">
							{deleteError}
						</p>
					)}
					<AlertDialogFooter>
						<AlertDialogCancel disabled={busy}>Keep profile</AlertDialogCancel>
						<Button
							variant="destructive"
							disabled={busy}
							onClick={() => void remove()}
						>
							{busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}Delete
							profile
						</Button>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}

export function ProfileTemplateStatus({
	message,
	retry,
}: { message: string; retry?: () => void }) {
	return (
		<div className="flex min-h-48 flex-col items-center justify-center gap-4 rounded-2xl border p-8 text-center">
			<output className="text-sm text-muted-foreground">{message}</output>
			{retry && (
				<Button variant="outline" onClick={retry}>
					Try again
				</Button>
			)}
		</div>
	);
}
