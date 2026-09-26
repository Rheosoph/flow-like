"use client";

import { useTranslation } from "@flow-like/locales";
import {
	keepPreviousData,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../../lib/api-error";
import { getApiOrigin } from "../../../../lib/api-url";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { isAzureBlobStorageUrl } from "../../../../lib/storage-url";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import type { ProfileMediaUpload } from "../../../profile-templates/profile-template-editor";
import { toExploreError } from "../../../store/explore/explore-model";
import type {
	ExploreEditorState,
	ExploreLayoutDoc,
	ExplorePlacementInput,
	ExploreSlotKey,
	ExploreViewer,
} from "../../../store/explore/explore-types";
import {
	type ExploreOrderInput,
	adminPreviewPath,
	errorMessage,
	errorStatus,
	mediaFormat,
	parseEditorState,
	parsePreview,
} from "./explore-admin-model";

/** PUTs an image to a signed storage URL; desktop injects a native HTTP implementation. */
export type AdminExploreMediaUpload = ProfileMediaUpload;

export const browserMediaUpload: AdminExploreMediaUpload = async (
	url,
	file,
) => {
	const headers: Record<string, string> = { "Content-Type": file.type };
	if (isAzureBlobStorageUrl(url)) headers["x-ms-blob-type"] = "BlockBlob";
	const response = await fetch(url, { method: "PUT", body: file, headers });
	if (!response.ok) throw new Error(`Image upload failed (${response.status})`);
};

/** Another admin changed the draft between our read and this write; the editor now holds their version. */
export class ExploreDraftConflictError extends Error {
	constructor(message: string) {
		super(message);
		this.name = "ExploreDraftConflictError";
	}
}

export function isDraftConflict(
	error: unknown,
): error is ExploreDraftConflictError {
	return error instanceof ExploreDraftConflictError;
}

interface SignedMedia {
	url?: string;
	finalUrl?: string | null;
}

function retryEditorRead(failureCount: number, error: unknown): boolean {
	const status = errorStatus(error);
	if (status !== undefined && status >= 400 && status < 500) return false;
	return failureCount < 1;
}

export function useAdminExplore() {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const auth = useAuth();
	const ready = useBackendReady();
	const queryClient = useQueryClient();
	const sub = auth?.user?.profile?.sub ?? "local";
	const origin = getApiOrigin(backend.profile);
	const identity = [origin, sub, backend.profile?.id, auth?.isAuthenticated];
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready,
		identity,
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		ready,
		identity,
	);
	const allowed = new GlobalPermission(
		info.data?.permission ?? 0,
	).hasPermission(GlobalPermission.WriteLandingPage);
	const profileId = profile.data?.id ?? "";
	const queryKey = useMemo(
		() => ["admin", "explore", origin, sub, profileId] as const,
		[origin, sub, profileId],
	);
	const profileRef = useRef<IProfile | undefined>(profile.data);
	profileRef.current = profile.data;

	const requireProfile = useCallback((): IProfile => {
		const current = profileRef.current;
		if (!current) {
			throw new Error(
				t(
					"exploreProfileUnavailable",
					"Your admin profile is not available. Reload and try again.",
				),
			);
		}
		return current;
	}, [t]);

	const loadState = useCallback(async () => {
		try {
			return parseEditorState(
				await backend.apiState.get<unknown>(requireProfile(), "admin/explore"),
			);
		} catch (error) {
			throw toExploreError(error);
		}
	}, [backend, requireProfile]);

	const state = useQuery<ExploreEditorState, Error>({
		queryKey,
		queryFn: loadState,
		enabled: ready && !!profile.data && allowed,
		staleTime: 0,
		retry: retryEditorRead,
	});

	const queue = useRef<Promise<unknown>>(Promise.resolve());
	const conflictEpoch = useRef(0);
	const [pending, setPending] = useState(0);
	const [conflict, setConflict] = useState<string | null>(null);
	const conflictMessage = useCallback(
		() =>
			t(
				"exploreConflictMessage",
				"Another admin changed the Explore draft while you were editing.",
			),
		[t],
	);

	/** Serializes writes. A step queued before a draft conflict is dropped instead of being sent against a
	 * revision the admin has not seen, so a queued Publish or Discard never acts on someone else's draft. */
	const run = useCallback(
		(
			send: (
				revision: string,
				admin: IProfile,
				layout: ExploreLayoutDoc,
			) => Promise<unknown>,
		): Promise<ExploreEditorState> => {
			const epoch = conflictEpoch.current;
			const task = queue.current
				.catch(() => undefined)
				.then(async () => {
					if (epoch !== conflictEpoch.current) {
						throw new ExploreDraftConflictError(conflictMessage());
					}
					const admin = requireProfile();
					const current =
						queryClient.getQueryData<ExploreEditorState>(queryKey);
					const revision = current?.draftRevision ?? "default";
					try {
						const next = parseEditorState(
							await send(revision, admin, current?.layout ?? { slots: [] }),
						);
						queryClient.setQueryData(queryKey, next);
						void queryClient.invalidateQueries({
							queryKey: [...queryKey, "preview"],
						});
						void queryClient.invalidateQueries({ queryKey: ["explore"] });
						return next;
					} catch (error) {
						if (errorStatus(error) !== 409) throw error;
						const fresh = await queryClient
							.fetchQuery({ queryKey, queryFn: loadState, staleTime: 0 })
							.catch(() => undefined);
						if (fresh && fresh.draftRevision === revision) throw error;
						conflictEpoch.current++;
						const message = conflictMessage();
						setConflict(message);
						void queryClient.invalidateQueries({
							queryKey: [...queryKey, "preview"],
						});
						throw new ExploreDraftConflictError(errorMessage(error, message));
					}
				});
			queue.current = task;
			setPending((count) => count + 1);
			void task
				.catch(() => undefined)
				.finally(() => setPending((count) => count - 1));
			return task;
		},
		[conflictMessage, loadState, queryClient, queryKey, requireProfile],
	);

	const create = useCallback(
		(
			slotKey: ExploreSlotKey,
			placement: ExplorePlacementInput,
			position?: number,
		) =>
			run((expectedRevision, admin) =>
				backend.apiState.post(admin, "admin/explore/placements", {
					expectedRevision,
					slotKey,
					...(position === undefined ? {} : { position }),
					placement,
				}),
			),
		[backend, run],
	);

	const update = useCallback(
		(id: string, placement: ExplorePlacementInput) =>
			run((expectedRevision, admin) =>
				backend.apiState.put(
					admin,
					`admin/explore/placements/${encodeURIComponent(id)}`,
					{ expectedRevision, placement },
				),
			),
		[backend, run],
	);

	const remove = useCallback(
		(id: string) =>
			run((expectedRevision, admin) =>
				backend.apiState.del(
					admin,
					`admin/explore/placements/${encodeURIComponent(id)}?expected_revision=${encodeURIComponent(expectedRevision)}`,
				),
			),
		[backend, run],
	);

	/** The body is built inside the queue from the latest draft, so a queued move never sends a stale permutation. */
	const order = useCallback(
		(build: (layout: ExploreLayoutDoc) => ExploreOrderInput) =>
			run((expectedRevision, admin, layout) =>
				backend.apiState.put(admin, "admin/explore/order", {
					expectedRevision,
					...build(layout),
				}),
			),
		[backend, run],
	);

	const publish = useCallback(
		() =>
			run((expectedRevision, admin) =>
				backend.apiState.post(admin, "admin/explore/publish", {
					expectedRevision,
				}),
			),
		[backend, run],
	);

	const discard = useCallback(
		() =>
			run((expectedRevision, admin) =>
				backend.apiState.post(admin, "admin/explore/discard", {
					expectedRevision,
				}),
			),
		[backend, run],
	);

	const signMedia = useCallback(
		async (format: string) => {
			const signed = await backend.apiState.get<SignedMedia>(
				requireProfile(),
				`admin/explore/media?format=${format}`,
			);
			return {
				url: typeof signed?.url === "string" ? signed.url : "",
				finalUrl:
					typeof signed?.finalUrl === "string" && signed.finalUrl
						? signed.finalUrl
						: null,
			};
		},
		[backend, requireProfile],
	);

	const reload = useCallback(async () => {
		setConflict(null);
		return queryClient.fetchQuery({
			queryKey,
			queryFn: loadState,
			staleTime: 0,
		});
	}, [loadState, queryClient, queryKey]);

	return {
		backend,
		profile,
		info,
		ready,
		allowed,
		queryKey,
		state,
		saving: pending > 0,
		conflict,
		conflictMessage,
		clearConflict: () => setConflict(null),
		reload,
		create,
		update,
		remove,
		order,
		publish,
		discard,
		signMedia,
	};
}

export type AdminExploreApi = ReturnType<typeof useAdminExplore>;

export function useAdminExplorePreview(
	api: AdminExploreApi,
	viewer: ExploreViewer,
) {
	const { t } = useTranslation("admin");
	return useQuery({
		queryKey: [...api.queryKey, "preview", viewer],
		queryFn: async () => {
			const admin = api.profile.data;
			if (!admin) {
				throw new Error(
					t(
						"exploreProfileUnavailable",
						"Your admin profile is not available. Reload and try again.",
					),
				);
			}
			return parsePreview(
				await api.backend.apiState.get<unknown>(
					admin,
					adminPreviewPath(viewer),
				),
			);
		},
		enabled: api.allowed && !!api.profile.data && api.state.isSuccess,
		placeholderData: keepPreviousData,
		retry: retryEditorRead,
	});
}

/** Custom artwork needs a CDN: without one the media endpoint signs no public URL. */
export function useArtworkUpload(
	api: AdminExploreApi,
	uploadMedia: AdminExploreMediaUpload,
	enabled: boolean,
) {
	const { t } = useTranslation("admin");
	const probe = useQuery({
		queryKey: [...api.queryKey, "media"],
		queryFn: () => api.signMedia("webp"),
		enabled: enabled && api.allowed && !!api.profile.data,
		staleTime: Number.POSITIVE_INFINITY,
		retry: false,
	});
	const upload = useCallback(
		async (file: Blob) => {
			const format = mediaFormat(file.type);
			if (!format) {
				throw new Error(
					t("exploreArtworkFormat", "Use a PNG, JPEG or WebP image."),
				);
			}
			const failed = t(
				"exploreArtworkUploadFailed",
				"The image could not be uploaded. Try again.",
			);
			const signed = await api.signMedia(format).catch((error: unknown) => {
				throw new Error(apiErrorMessage(error, failed));
			});
			if (!signed.url || !signed.finalUrl) {
				throw new Error(
					t(
						"exploreArtworkNoCdn",
						"This hub has no CDN for Explore artwork. Use the item cover instead.",
					),
				);
			}
			await uploadMedia(signed.url, file).catch(() => {
				throw new Error(failed);
			});
			return signed.finalUrl;
		},
		[api, t, uploadMedia],
	);
	return {
		available: probe.data ? probe.data.finalUrl !== null : undefined,
		upload,
	};
}
