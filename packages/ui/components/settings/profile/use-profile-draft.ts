"use client";

import { useEffect, useRef, useSyncExternalStore } from "react";
import type { ISettingsProfile } from "../../../types";
import {
	ProfileDraftController,
	profileDraftSession,
	releaseProfileDraftSession,
} from "./profile-draft";

export function useProfileDraft(
	source: ISettingsProfile | undefined,
	save: (profile: ISettingsProfile) => Promise<void>,
	scope?: string,
) {
	const controllerRef = useRef<{
		scope: string | undefined;
		controller: ProfileDraftController;
		saveRef: { current: (profile: ISettingsProfile) => Promise<void> };
	} | null>(null);
	if (!controllerRef.current || controllerRef.current.scope !== scope) {
		const saveRef = { current: save };
		const handler = (profile: ISettingsProfile) => saveRef.current(profile);
		controllerRef.current = {
			scope,
			saveRef,
			controller: scope
				? profileDraftSession(scope, handler)
				: new ProfileDraftController(handler),
		};
	}
	const { controller, saveRef } = controllerRef.current;
	saveRef.current = save;
	controller.setSaveHandler((profile) => saveRef.current(profile));
	const snapshot = useSyncExternalStore(
		controller.subscribe,
		controller.getSnapshot,
		controller.getSnapshot,
	);

	useEffect(() => {
		if (source) controller.setSource(source);
	}, [source, controller]);
	useEffect(() => {
		const beforeUnload = (event: BeforeUnloadEvent) => {
			if (!controller.hasUnsaved()) return;
			event.preventDefault();
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", beforeUnload);
		return () => {
			window.removeEventListener("beforeunload", beforeUnload);
			void controller.flushAll().then(() => {
				if (scope) releaseProfileDraftSession(scope, controller);
			});
		};
	}, [controller, scope]);

	return {
		...snapshot,
		update: controller.update,
		retry: controller.retry,
		flush: () => controller.flush(),
		forget: (id: string) => controller.forget(id),
	};
}
