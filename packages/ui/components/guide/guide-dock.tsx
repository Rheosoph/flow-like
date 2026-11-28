"use client";

import { useEffect } from "react";
import { create } from "zustand";
import { useGuideStore } from "../../state/guide-state";
import { useSidebar } from "../ui/sidebar";
import { GuideCard } from "./guide-card";

const useDockRegistry = create<{
	docks: number;
	add: () => void;
	remove: () => void;
}>((set) => ({
	docks: 0,
	add: () => set((state) => ({ docks: state.docks + 1 })),
	remove: () => set((state) => ({ docks: Math.max(0, state.docks - 1) })),
}));

/** True while a sidebar shows the guide, so the floating card stays away. */
export function useGuideDocked(): boolean {
	return useDockRegistry((state) => state.docks > 0);
}

/**
 * The guide's place in an app sidebar. It only docks while the sidebar is open
 * on a wide screen; otherwise the provider floats the same card.
 */
export function GuideDock() {
	const { state, isMobile } = useSidebar();
	const visible = state === "expanded" && !isMobile;
	const active = useGuideStore(
		(guide) => guide.run !== null || guide.finished !== null,
	);
	const add = useDockRegistry((registry) => registry.add);
	const remove = useDockRegistry((registry) => registry.remove);

	useEffect(() => {
		if (!visible) return;
		add();
		return remove;
	}, [visible, add, remove]);

	if (!visible || !active) return null;
	return (
		<div className="px-1 pb-2">
			<GuideCard variant="dock" />
		</div>
	);
}
