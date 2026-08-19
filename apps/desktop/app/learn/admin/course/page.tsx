"use client";
import {
	AppLinkPicker,
	type AppOption,
	AssetsEditor,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	CourseForm,
	type CourseFormValue,
	Input,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { Trans, useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ChevronRight, Pencil, Plus, Trash2 } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { learnApi } from "../../../../lib/learn-api";

export default function CourseAdminPage() {
	return (
		<Suspense fallback={null}>
			<CourseAdminContent />
		</Suspense>
	);
}

function CourseAdminContent() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const searchParams = useSearchParams();
	const courseId = searchParams.get("courseId") ?? "";
	const router = useRouter();
	const queryClient = useQueryClient();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for course authoring.");
		}
		return profile;
	};

	const structureQuery = useQuery({
		queryKey: ["learn", "admin", "structure", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => learnApi.getCourseStructure(getProfile(), auth, courseId),
	});

	const linksQuery = useQuery({
		queryKey: ["learn", "admin", "links", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => learnApi.listAppLinks(getProfile(), auth, courseId),
	});

	const appsQuery = useInvoke(backend.appState.getApps, backend.appState, []);

	const appOptions: AppOption[] = useMemo(() => {
		const data = (appsQuery.data ?? []) as Array<unknown>;
		return data
			.map((entry) => {
				if (Array.isArray(entry) && entry.length >= 1) {
					const app = entry[0] as { id?: string };
					const meta = entry[1] as { name?: string } | undefined;
					if (!app?.id) return null;
					return { id: app.id, name: meta?.name ?? app.id };
				}
				const app = entry as { id?: string; name?: string };
				if (!app?.id) return null;
				return { id: app.id, name: app.name ?? app.id };
			})
			.filter((x): x is AppOption => Boolean(x));
	}, [appsQuery.data]);

	const upsertCourse = useMutation({
		mutationFn: (value: CourseFormValue) =>
			learnApi.upsertCourse(getProfile(), auth, courseId, {
				language: value.language,
				slug: value.slug,
				difficulty: value.difficulty,
				category: value.category,
				estimated_minutes: value.estimated_minutes,
				is_published: value.is_published,
				tags: [...value.tags],
				position: value.position,
				name: value.name,
				description: value.description,
				long_description: value.long_description,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "structure", courseId],
			});
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "courses"],
			});
			queryClient.invalidateQueries({ queryKey: ["learn", "courses"] });
			toast.success("Course saved");
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not save course");
		},
	});

	const uploadCourseMedia = useMutation({
		mutationFn: (args: { item: "icon" | "thumbnail"; file: File }) =>
			learnApi.uploadCourseMedia(
				getProfile(),
				auth,
				courseId,
				args.item,
				args.file,
				structureQuery.data?.course?.language ?? "en",
			),
		onSuccess: (_, args) => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "structure", courseId],
			});
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "courses"],
			});
			queryClient.invalidateQueries({ queryKey: ["learn", "courses"] });
			toast.success(
				args.item === "icon"
					? t("courseIconUploaded", "Course icon uploaded")
					: t("courseBannerUploaded", "Course banner uploaded"),
			);
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not upload course media");
		},
	});

	const deleteCourse = useMutation({
		mutationFn: () => learnApi.deleteCourse(getProfile(), auth, courseId),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "courses"],
			});
			toast.success("Course deleted");
			router.push("/learn/admin");
		},
	});

	const course = structureQuery.data?.course ?? null;
	const modules = structureQuery.data?.modules ?? [];

	if (!courseId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<Card>
						<CardHeader>
							<CardTitle>{t("courseMissing", "Course missing")}</CardTitle>
							<CardDescription>
								{`Open a course from the course admin library.`}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button asChild variant="outline">
								<Link href="/learn/admin">
									<ArrowLeft className="mr-2 h-4 w-4" />
									{t("courseAdmin", "Course admin")}
								</Link>
							</Button>
						</CardContent>
					</Card>
				</div>
			</div>
		);
	}

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-5xl p-6 md:p-8 lg:p-10 space-y-6">
				<div className="flex items-center gap-3">
					<Link
						href="/learn/admin"
						className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
					>
						<ArrowLeft className="h-3 w-3" />
						{t("authoring", "Authoring")}
					</Link>
					<h1 className="ml-auto text-2xl font-semibold truncate max-w-[60%]">
						{course?.name ?? courseId}
					</h1>
					<Button
						variant="ghost"
						size="sm"
						onClick={() => {
							if (confirm("Delete this course and all its content?")) {
								deleteCourse.mutate();
							}
						}}
					>
						<Trash2 className="h-3.5 w-3.5 mr-1.5" />
						{t("delete", "Delete")}
					</Button>
				</div>

				<Card>
					<CardHeader>
						<CardTitle className="text-base">
							{t("courseDetails", "Course details")}
						</CardTitle>
					</CardHeader>
					<CardContent>
						<CourseForm
							initial={course}
							onSubmit={async (v) => {
								await upsertCourse.mutateAsync(v);
							}}
							submitting={upsertCourse.isPending}
							submitLabel="Save course"
							onUploadMedia={async (item, file) => {
								await uploadCourseMedia.mutateAsync({ item, file });
							}}
							mediaUploading={
								uploadCourseMedia.isPending
									? (uploadCourseMedia.variables?.item ?? null)
									: null
							}
						/>
					</CardContent>
				</Card>

				<ModulesEditor
					courseId={courseId}
					modules={modules}
					onChanged={() =>
						queryClient.invalidateQueries({
							queryKey: ["learn", "admin", "structure", courseId],
						})
					}
				/>

				<AppLinksEditor
					courseId={courseId}
					links={linksQuery.data ?? []}
					appOptions={appOptions}
					onChanged={() =>
						queryClient.invalidateQueries({
							queryKey: ["learn", "admin", "links", courseId],
						})
					}
				/>

				<AssetsEditor
					api={learnApi}
					profile={profile}
					auth={auth}
					courseId={courseId}
				/>
			</div>
		</div>
	);
}

interface ModulesEditorProps {
	readonly courseId: string;
	readonly modules: ReadonlyArray<{
		readonly id: string;
		readonly title: string;
		readonly description: string | null;
		readonly position: number;
		readonly lessons: ReadonlyArray<{
			readonly id: string;
			readonly title: string;
			readonly position: number;
			readonly is_optional: boolean;
		}>;
	}>;
	readonly onChanged: () => void;
}

function ModulesEditor({ courseId, modules, onChanged }: ModulesEditorProps) {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const router = useRouter();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for course authoring.");
		}
		return profile;
	};
	const [newTitle, setNewTitle] = useState("");
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editTitle, setEditTitle] = useState("");

	const createModule = useMutation({
		mutationFn: () => {
			const id = `mod-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
			return learnApi.upsertModule(getProfile(), auth, courseId, id, {
				title: newTitle.trim() || "New module",
				description: null,
				position: modules.length,
			});
		},
		onSuccess: () => {
			setNewTitle("");
			onChanged();
		},
	});

	const renameModule = useMutation({
		mutationFn: async ({
			moduleId,
			title,
		}: {
			moduleId: string;
			title: string;
		}) => {
			const m = modules.find((x) => x.id === moduleId);
			if (!m) throw new Error("Module not found");
			await learnApi.upsertModule(getProfile(), auth, courseId, moduleId, {
				title,
				description: m.description,
				position: m.position,
			});
		},
		onSuccess: () => {
			setEditingId(null);
			setEditTitle("");
			onChanged();
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not rename module");
		},
	});

	const deleteModule = useMutation({
		mutationFn: (moduleId: string) =>
			learnApi.deleteModule(getProfile(), auth, courseId, moduleId),
		onSuccess: () => onChanged(),
	});

	const createLesson = useMutation({
		mutationFn: async ({
			moduleId,
			position,
		}: {
			moduleId: string;
			position: number;
		}) => {
			const lessonId = `les-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
			await learnApi.upsertLesson(
				getProfile(),
				auth,
				courseId,
				moduleId,
				lessonId,
				{
					title: t("newLesson", "New lesson"),
					content: t("newLessonStartWriting", "# New lesson Start writing…"),
					position,
				},
			);
			return { moduleId, lessonId };
		},
		onSuccess: ({ moduleId, lessonId }) => {
			onChanged();
			router.push(
				`/learn/admin/lesson?courseId=${encodeURIComponent(courseId)}&moduleId=${encodeURIComponent(moduleId)}&lessonId=${encodeURIComponent(lessonId)}`,
			);
		},
	});

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("modulesLessons", "Modules & lessons")}
				</CardTitle>
				<CardDescription>
					{t(
						"modulesGroupLessonsReorderViaThePositionField",
						"Modules group lessons. Reorder via the position field.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<form
					className="flex items-end gap-2"
					onSubmit={(e) => {
						e.preventDefault();
						createModule.mutate();
					}}
				>
					<div className="flex-1 space-y-1.5">
						<label
							htmlFor="new-module-title"
							className="text-xs font-medium text-muted-foreground"
						>
							{t("newModuleTitle", "New module title")}
						</label>
						<Input
							id="new-module-title"
							value={newTitle}
							onChange={(e) => setNewTitle(e.target.value)}
							placeholder={t("gettingStarted", "Getting started")}
						/>
					</div>
					<Button type="submit" disabled={createModule.isPending || !profile}>
						<Plus className="h-4 w-4 mr-2" />
						{t("addModule", "Add module")}
					</Button>
				</form>

				{modules.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t("noModulesYet", "No modules yet.")}
					</p>
				) : (
					<ul className="space-y-3">
						{modules.map((m) => (
							<li key={m.id} className="rounded-md border p-3 space-y-2">
								<div className="flex items-center gap-2">
									<span className="text-xs font-mono text-muted-foreground">{`#${m.position}`}</span>
									{editingId === m.id ? (
										<form
											className="flex flex-1 items-center gap-2"
											onSubmit={(e) => {
												e.preventDefault();
												const trimmed = editTitle.trim();
												if (!trimmed || trimmed === m.title) {
													setEditingId(null);
													return;
												}
												renameModule.mutate({
													moduleId: m.id,
													title: trimmed,
												});
											}}
										>
											<Input
												autoFocus
												value={editTitle}
												onChange={(e) => setEditTitle(e.target.value)}
												onKeyDown={(e) => {
													if (e.key === "Escape") {
														e.preventDefault();
														setEditingId(null);
													}
												}}
												className="h-8"
												disabled={renameModule.isPending}
											/>
											<Button
												type="submit"
												size="sm"
												disabled={renameModule.isPending || !editTitle.trim()}
											>
												{t("save", "Save")}
											</Button>
											<Button
												type="button"
												variant="ghost"
												size="sm"
												onClick={() => setEditingId(null)}
												disabled={renameModule.isPending}
											>
												{t("cancel", "Cancel")}
											</Button>
										</form>
									) : (
										<>
											<span className="font-medium flex-1 truncate">
												{m.title}
											</span>
											<Button
												variant="outline"
												size="sm"
												onClick={() => {
													setEditingId(m.id);
													setEditTitle(m.title);
												}}
												disabled={!profile}
											>
												<Pencil className="h-3 w-3" />
											</Button>
											<Button
												variant="ghost"
												size="sm"
												onClick={() => {
													if (confirm("Delete this module?")) {
														deleteModule.mutate(m.id);
													}
												}}
											>
												<Trash2 className="h-3 w-3" />
											</Button>
										</>
									)}
								</div>
								<ul className="ml-4 space-y-1">
									{m.lessons.map((l) => (
										<li key={l.id}>
											<button
												type="button"
												className="flex items-center gap-2 w-full text-left text-sm py-1 hover:bg-accent/50 rounded px-2"
												onClick={() =>
													router.push(
														`/learn/admin/lesson?courseId=${encodeURIComponent(courseId)}&moduleId=${encodeURIComponent(m.id)}&lessonId=${encodeURIComponent(l.id)}`,
													)
												}
											>
												<span className="text-xs font-mono text-muted-foreground w-6">{`#${l.position}`}</span>
												<span className="flex-1">{l.title}</span>
												{l.is_optional && (
													<Badge variant="outline" className="text-[10px]">
														optional
													</Badge>
												)}
												<ChevronRight className="h-3 w-3 text-muted-foreground" />
											</button>
										</li>
									))}
									<li>
										<Button
											variant="ghost"
											size="sm"
											className="w-full justify-start text-muted-foreground"
											onClick={() =>
												createLesson.mutate({
													moduleId: m.id,
													position: m.lessons.length,
												})
											}
											disabled={createLesson.isPending}
										>
											<Plus className="h-3 w-3 mr-2" />
											{t("addLesson", "Add lesson")}
										</Button>
									</li>
								</ul>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}

interface AppLinksEditorProps {
	readonly courseId: string;
	readonly links: ReadonlyArray<{
		readonly id: string;
		readonly app_id: string;
		readonly purpose: string;
		readonly alias: string | null;
	}>;
	readonly appOptions: ReadonlyArray<AppOption>;
	readonly onChanged: () => void;
}

function AppLinksEditor({
	courseId,
	links,
	appOptions,
	onChanged,
}: AppLinksEditorProps) {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for course authoring.");
		}
		return profile;
	};
	const [draft, setDraft] = useState({
		appId: "",
		alias: "",
		purpose: "SHARED_TEMPLATE",
	});

	const upsert = useMutation({
		mutationFn: async (linkId: string | null) => {
			const id =
				linkId ??
				`link-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
			await learnApi.upsertAppLink(getProfile(), auth, courseId, id, {
				app_id: draft.appId,
				alias: draft.alias || null,
				purpose: draft.purpose,
			});
		},
		onSuccess: () => {
			setDraft({ appId: "", alias: "", purpose: "SHARED_TEMPLATE" });
			onChanged();
		},
	});

	const remove = useMutation({
		mutationFn: (linkId: string) =>
			learnApi.deleteAppLink(getProfile(), auth, courseId, linkId),
		onSuccess: () => onChanged(),
	});

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("linkedApps", "Linked apps")}
				</CardTitle>
				<CardDescription>
					<Trans i18nKey="appsUsedByThisCourseStrongsharedTemplatestrongPlaygroundAppsAreForkedIntoTheUsersLibraryOnFirstEncounter">
						Apps used by this course. <strong>Shared template</strong> /
						Playground apps are forked into the user's library on first
						encounter.
					</Trans>{" "}
					<Trans i18nKey="strongreferencestrongAppsAreReadonlyLinks">
						<strong>Reference</strong> apps are read-only links.
					</Trans>
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-2 rounded-md border p-3 bg-muted/20">
					<AppLinkPicker apps={appOptions} value={draft} onChange={setDraft} />
					<Button
						onClick={() => upsert.mutate(null)}
						disabled={!draft.appId || upsert.isPending || !profile}
					>
						<Plus className="h-4 w-4 mr-2" />
						{t("addLink", "Add link")}
					</Button>
				</div>

				{links.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t("noAppsLinkedYet", "No apps linked yet.")}
					</p>
				) : (
					<ul className="space-y-2">
						{links.map((l) => (
							<li
								key={l.id}
								className="flex items-center gap-3 rounded-md border px-3 py-2"
							>
								<Badge variant="outline">{l.alias ?? "(no alias)"}</Badge>
								<code className="text-xs text-muted-foreground flex-1 truncate">
									{l.app_id}
								</code>
								<Badge variant="secondary">{l.purpose}</Badge>
								<Button
									variant="ghost"
									size="sm"
									onClick={() => {
										if (confirm("Remove this app link?")) {
											remove.mutate(l.id);
										}
									}}
								>
									<Trash2 className="h-3 w-3" />
								</Button>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}
