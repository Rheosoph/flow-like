"use client";

import { lazy } from "react";

/** Loaded when a bot's Files or Data pane first opens; both pull in large parts of the app. */
export const WorkspaceStorage = lazy(() =>
	import("../storage/storage-system").then((module) => ({
		default: module.StorageSystem,
	})),
);

export const WorkspaceDataStudio = lazy(() =>
	import("../settings/explore/explore-page").then((module) => ({
		default: module.DataStudioPage,
	})),
);
