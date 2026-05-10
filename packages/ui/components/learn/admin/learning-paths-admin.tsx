"use client";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Plus, Route, Trash2, X } from "lucide-react";
import { useMemo, useState } from "react";
import type { AuthContextProps } from "react-oidc-context";
import { toast } from "sonner";
import type { CourseListItem, LearningPath } from "../../../lib/learn/types";
import type { IProfile } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";

/**
 * Minimal contract the admin section needs from a `learnApi` instance.
 * Both `apps/desktop/lib/learn-api.ts` and `apps/web/lib/learn-api.ts`
 * already satisfy this — keeping it narrow lets the section ship via
 * packages/ui without pulling in the rest of the API surface.
 */
export interface LearningPathsAdminApi {
	listLearningPaths(
		profile: IProfile,
		auth: AuthContextProps,
		opts?: { language?: string; includeUnpublished?: boolean },
	): Promise<LearningPath[]>;
	upsertLearningPath(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		body: {
			title: string;
			slug?: string | null;
			description?: string | null;
			position?: number;
			is_published?: boolean;
		},
	): Promise<LearningPath>;
	deleteLearningPath(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
	): Promise<void>;
	upsertLearningPathStep(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		courseId: string,
		body: { position: number },
	): Promise<void>;
	deleteLearningPathStep(
		profile: IProfile,
		auth: AuthContextProps,
		pathId: string,
		courseId: string,
	): Promise<void>;
}

export interface LearningPathsAdminProps {
	readonly api: LearningPathsAdminApi;
	readonly profile: IProfile | null;
	readonly auth: AuthContextProps;
	readonly courses: ReadonlyArray<CourseListItem>;
	readonly canManage: boolean;
}

/**
 * Admin section that lists curated learning paths and lets editors
 * sequence existing courses into them. Lifted out of
 * `apps/desktop/app/learn/admin/page.tsx` so both desktop and web
 * can share the same implementation — the host only supplies the
 * api caller, the auth/profile context, and the course catalog used
 * to populate the "add course" dropdown.
 */
export function LearningPathsAdmin({
	api,
	profile,
	auth,
	courses,
	canManage,
}: LearningPathsAdminProps) {
	const queryClient = useQueryClient();
	const profileId = profile?.id ?? "no-profile";
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for path admin.");
		}
		return profile;
	};

	const pathsQuery = useQuery({
		queryKey: ["learn", "admin", "paths", profileId],
		enabled: Boolean(profile),
		queryFn: () =>
			api.listLearningPaths(getProfile(), auth, {
				includeUnpublished: true,
			}),
	});

	const invalidate = () => {
		queryClient.invalidateQueries({ queryKey: ["learn", "admin", "paths"] });
		queryClient.invalidateQueries({ queryKey: ["learn", "paths"] });
	};

	const [newTitle, setNewTitle] = useState("");
	const createMutation = useMutation({
		mutationFn: () => {
			const id = `path-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
			return api.upsertLearningPath(getProfile(), auth, id, {
				title: newTitle.trim() || "New learning path",
				description: null,
				is_published: false,
				position: pathsQuery.data?.length ?? 0,
			});
		},
		onSuccess: () => {
			setNewTitle("");
			invalidate();
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not create path");
		},
	});

	const paths = pathsQuery.data ?? [];

	return (
		<div className="space-y-3">
			<div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
				<div className="flex items-center gap-2">
					<Route className="h-4 w-4 text-violet-400" />
					<div>
						<h2 className="text-lg font-semibold">Learning paths</h2>
						<p className="text-sm text-muted-foreground">
							Curated journeys — sequence existing courses into a path.
						</p>
					</div>
				</div>
			</div>

			{canManage && (
				<Card className="border-border/60 bg-card/80">
					<CardContent className="pt-6">
						<form
							className="grid grid-cols-1 items-end gap-3 md:grid-cols-[1fr_auto]"
							onSubmit={(e) => {
								e.preventDefault();
								createMutation.mutate();
							}}
						>
							<div className="space-y-1.5">
								<label
									htmlFor="new-path-title"
									className="text-xs font-medium text-muted-foreground"
								>
									New path title
								</label>
								<Input
									id="new-path-title"
									value={newTitle}
									onChange={(e) => setNewTitle(e.target.value)}
									placeholder="Build with Flow-Like, end-to-end"
								/>
							</div>
							<Button type="submit" disabled={createMutation.isPending}>
								<Plus className="mr-2 h-4 w-4" />
								Add path
							</Button>
						</form>
					</CardContent>
				</Card>
			)}

			{paths.length === 0 ? (
				<Card className="border-dashed bg-card/60">
					<CardHeader>
						<CardTitle className="text-base">No paths yet</CardTitle>
						<CardDescription>
							{canManage
								? "Create one above to start sequencing courses."
								: "Paths will appear here once an admin creates them."}
						</CardDescription>
					</CardHeader>
				</Card>
			) : (
				paths.map((path) => (
					<LearningPathRow
						key={path.id}
						api={api}
						profile={profile}
						auth={auth}
						path={path}
						courses={courses}
						canManage={canManage}
						onChanged={invalidate}
					/>
				))
			)}
		</div>
	);
}

interface LearningPathRowProps {
	readonly api: LearningPathsAdminApi;
	readonly profile: IProfile | null;
	readonly auth: AuthContextProps;
	readonly path: LearningPath;
	readonly courses: ReadonlyArray<CourseListItem>;
	readonly canManage: boolean;
	readonly onChanged: () => void;
}

function LearningPathRow({
	api,
	profile,
	auth,
	path,
	courses,
	canManage,
	onChanged,
}: LearningPathRowProps) {
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for path admin.");
		}
		return profile;
	};

	const [title, setTitle] = useState(path.title);
	const [description, setDescription] = useState(path.description ?? "");
	const [addCourseId, setAddCourseId] = useState("");

	const stepIds = useMemo(
		() => new Set(path.steps.map((s) => s.course_id)),
		[path.steps],
	);
	const addableCourses = useMemo(
		() => courses.filter((c) => !stepIds.has(c.id)),
		[courses, stepIds],
	);
	const sortedSteps = useMemo(
		() => [...path.steps].sort((a, b) => a.position - b.position),
		[path.steps],
	);

	const upsertMeta = useMutation({
		mutationFn: (body: {
			title?: string;
			description?: string | null;
			is_published?: boolean;
		}) =>
			api.upsertLearningPath(getProfile(), auth, path.id, {
				title: body.title ?? title,
				description:
					body.description === undefined
						? description.trim() || null
						: body.description,
				is_published: body.is_published ?? path.is_published,
				position: path.position,
			}),
		onSuccess: onChanged,
		onError: (err) => {
			console.error(err);
			toast.error("Could not save path");
		},
	});

	const deletePath = useMutation({
		mutationFn: () => api.deleteLearningPath(getProfile(), auth, path.id),
		onSuccess: onChanged,
		onError: (err) => {
			console.error(err);
			toast.error("Could not delete path");
		},
	});

	const upsertStep = useMutation({
		mutationFn: ({
			courseId,
			position,
		}: {
			courseId: string;
			position: number;
		}) =>
			api.upsertLearningPathStep(getProfile(), auth, path.id, courseId, {
				position,
			}),
		onSuccess: onChanged,
		onError: (err) => {
			console.error(err);
			toast.error("Could not update path step");
		},
	});

	const deleteStep = useMutation({
		mutationFn: (courseId: string) =>
			api.deleteLearningPathStep(getProfile(), auth, path.id, courseId),
		onSuccess: onChanged,
		onError: (err) => {
			console.error(err);
			toast.error("Could not remove step");
		},
	});

	const move = async (index: number, direction: -1 | 1) => {
		const target = index + direction;
		if (target < 0 || target >= sortedSteps.length) return;
		const a = sortedSteps[index];
		const b = sortedSteps[target];
		if (!a || !b) return;
		await upsertStep.mutateAsync({
			courseId: a.course_id,
			position: b.position,
		});
		await upsertStep.mutateAsync({
			courseId: b.course_id,
			position: a.position,
		});
	};

	const addCourse = () => {
		if (!addCourseId) return;
		void upsertStep.mutateAsync({
			courseId: addCourseId,
			position: sortedSteps.length,
		});
		setAddCourseId("");
	};

	return (
		<Card className="border-border/60 bg-card/75">
			<CardHeader className="space-y-3">
				<div className="flex flex-wrap items-center gap-2">
					<Input
						value={title}
						onChange={(e) => setTitle(e.target.value)}
						onBlur={() => {
							if (title.trim() && title !== path.title) {
								upsertMeta.mutate({ title: title.trim() });
							}
						}}
						disabled={!canManage}
						className="max-w-md flex-1 text-base font-semibold"
					/>
					<Badge variant={path.is_published ? "default" : "outline"}>
						{path.is_published ? "Published" : "Draft"}
					</Badge>
					{canManage && (
						<>
							<div className="flex items-center gap-2 text-xs text-muted-foreground">
								<span>Published</span>
								<Switch
									checked={path.is_published}
									onCheckedChange={(checked) =>
										upsertMeta.mutate({ is_published: checked })
									}
								/>
							</div>
							<Button
								variant="ghost"
								size="sm"
								onClick={() => {
									if (
										confirm(
											"Delete this learning path? Courses themselves are kept.",
										)
									) {
										deletePath.mutate();
									}
								}}
							>
								<Trash2 className="h-3.5 w-3.5" />
							</Button>
						</>
					)}
				</div>
				<Textarea
					value={description}
					onChange={(e) => setDescription(e.target.value)}
					onBlur={() => {
						const next = description.trim() || null;
						if (next !== (path.description ?? null)) {
							upsertMeta.mutate({ description: next });
						}
					}}
					disabled={!canManage}
					placeholder="Short description shown in the catalog…"
					className="min-h-15"
				/>
			</CardHeader>
			<CardContent className="space-y-3">
				{sortedSteps.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						No courses yet — add one below.
					</p>
				) : (
					<ol className="space-y-2">
						{sortedSteps.map((step, i) => (
							<li
								key={step.course_id}
								className="flex items-center gap-2 rounded-lg border border-border/50 bg-background/40 p-2"
							>
								<span className="grid size-7 shrink-0 place-items-center rounded-full bg-violet-500/15 text-xs font-semibold text-violet-300 ring-1 ring-violet-400/30">
									{i + 1}
								</span>
								<span className="min-w-0 flex-1 truncate text-sm">
									{step.course?.name ?? step.course_id}
								</span>
								{canManage && (
									<>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => {
												void move(i, -1);
											}}
											disabled={i === 0}
										>
											<ArrowUp className="h-3.5 w-3.5" />
										</Button>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => {
												void move(i, 1);
											}}
											disabled={i === sortedSteps.length - 1}
										>
											<ArrowDown className="h-3.5 w-3.5" />
										</Button>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => deleteStep.mutate(step.course_id)}
										>
											<X className="h-3.5 w-3.5" />
										</Button>
									</>
								)}
							</li>
						))}
					</ol>
				)}

				{canManage && addableCourses.length > 0 && (
					<div className="flex gap-2">
						<select
							value={addCourseId}
							onChange={(e) => setAddCourseId(e.target.value)}
							className="flex h-9 flex-1 rounded-md border border-input bg-background px-3 text-sm"
						>
							<option value="">Pick a course to add…</option>
							{addableCourses.map((c) => (
								<option key={c.id} value={c.id}>
									{c.name ?? c.id}
								</option>
							))}
						</select>
						<Button onClick={addCourse} disabled={!addCourseId}>
							<Plus className="mr-1.5 h-4 w-4" />
							Add course
						</Button>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
