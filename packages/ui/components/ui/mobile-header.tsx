"use client";
import { SidebarTrigger } from "@flow-like/flow-like-ui";
import { createId } from "@paralleldrive/cuid2";
import React, {
	createContext,
	useCallback,
	useContext,
	useMemo,
	useRef,
	useState,
	useEffect,
} from "react";

export type MobileHeaderControls = {
	title?: React.ReactNode;
	left?: React.ReactNode | React.ReactNode[];
	right?: React.ReactNode | React.ReactNode[];
};

type Ctx = {
	register: (id: string, controls: MobileHeaderControls) => void;
	update: (id: string, controls: MobileHeaderControls) => void;
	unregister: (id: string) => void;
	active: MobileHeaderControls | null;
};

const MobileHeaderContext = createContext<Ctx | null>(null);

export const MobileHeaderProvider: React.FC<{
	children: React.ReactNode;
}> = ({ children }) => {
	const [controlsMap, setControlsMap] = useState<
		Map<string, MobileHeaderControls>
	>(new Map());

	const register = useCallback((id: string, controls: MobileHeaderControls) => {
		setControlsMap((prev) => {
			const next = new Map(prev);
			next.set(id, controls);
			return next;
		});
	}, []);

	const update = useCallback((id: string, controls: MobileHeaderControls) => {
		setControlsMap((prev) => {
			const next = new Map(prev);
			const existing = next.get(id) ?? {};
			next.set(id, { ...existing, ...controls });
			return next;
		});
	}, []);

	const unregister = useCallback((id: string) => {
		setControlsMap((prev) => {
			if (!prev.has(id)) return prev;
			const next = new Map(prev);
			next.delete(id);
			return next;
		});
	}, []);

	const active = useMemo<MobileHeaderControls | null>(() => {
		if (controlsMap.size === 0) return null;
		const last = Array.from(controlsMap.values()).at(-1) ?? null;
		return last ?? null;
	}, [controlsMap]);

	const value = useMemo<Ctx>(
		() => ({ register, update, unregister, active }),
		[register, update, unregister, active],
	);

	return (
		<MobileHeaderContext.Provider value={value}>
			{children}
		</MobileHeaderContext.Provider>
	);
};

export function useMobileHeader(
	controls?: MobileHeaderControls,
	deps: React.DependencyList = [],
) {
	const ctx = useContext(MobileHeaderContext);
	if (!ctx)
		throw new Error("useMobileHeader must be used within MobileHeaderProvider");
	const register = ctx.register;
	const updateInCtx = ctx.update;
	const unregister = ctx.unregister;
	const idRef = useRef<string | null>(null);

	const ensureId = useCallback(() => {
		if (!idRef.current) idRef.current = createId();
		return idRef.current;
	}, []);

	useEffect(() => {
		if (!controls) return;
		const id = ensureId();
		register(id, controls);
		return () => unregister(id);
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, deps);

	// Always clean up on unmount (covers update/set calls without initial controls)
	useEffect(() => {
		return () => {
			if (idRef.current) {
				unregister(idRef.current);
			}
		};
	}, [unregister]);

	const set = useCallback(
		(next: MobileHeaderControls) => {
			const id = ensureId();
			register(id, next);
			return () => unregister(id);
		},
		[ensureId, register, unregister],
	);

	const update = useCallback(
		(next: MobileHeaderControls) => {
			const id = ensureId();
			updateInCtx(id, next);
		},
		[ensureId, updateInCtx],
	);

	const clear = useCallback(() => {
		if (!idRef.current) return;
		unregister(idRef.current);
	}, [unregister]);

	return { set, update, clear } as const;
}

export const MobileHeader: React.FC<{
	showSidebarTrigger?: boolean;
}> = ({ showSidebarTrigger = true }) => {
	const ctx = useContext(MobileHeaderContext);
	const active = ctx?.active ?? null;
	const ref = React.useRef<HTMLDivElement | null>(null);
	const hasAdditionalContent = useMemo(
		() =>
			[active?.title, active?.left, active?.right].some(
				(content) => React.Children.toArray(content).length > 0,
			),
		[active?.title, active?.left, active?.right],
	);
	const shouldHide = !showSidebarTrigger && !hasAdditionalContent;

	useEffect(() => {
		if (shouldHide) {
			document.documentElement.style.setProperty(
				"--mobile-header-height",
				"0px",
			);
			return;
		}

		const el = ref.current;
		if (!el) return;
		const setVar = () => {
			const h = el.offsetHeight || 56;
			document.documentElement.style.setProperty(
				"--mobile-header-height",
				`${h}px`,
			);
		};
		setVar();
		const ro = new ResizeObserver(setVar);
		ro.observe(el);
		return () => ro.disconnect();
	}, [shouldHide]);

	const left = useMemo(() => {
		if (!active?.left) return null;
		return Array.isArray(active.left) ? active.left : [active.left];
	}, [active?.left]);

	const right = useMemo(() => {
		if (!active?.right) return null;
		return Array.isArray(active.right) ? active.right : [active.right];
	}, [active?.right]);

	const leftNodes = left?.map((node, i) => (
		<React.Fragment key={i}>{node}</React.Fragment>
	));

	const rightNodes = right?.map((node, i) => (
		<React.Fragment key={i}>{node}</React.Fragment>
	));

	if (shouldHide) return null;

	return (
		<div ref={ref} className="md:hidden sticky top-0 z-40 px-2 pt-2 pb-1">
			<div className="rounded-xl bg-card/80 shadow-2xl flex items-center justify-between gap-2 p-2">
				<div className="flex items-center gap-2 min-w-0">
					{showSidebarTrigger && (
						<SidebarTrigger
							className="size-10 rounded-lg border extend-touch-target"
							aria-label="Open Menu"
						/>
					)}
					{leftNodes}
				</div>
				<div className="flex-1 min-w-0 text-center font-medium truncate">
					{active?.title ?? null}
				</div>
				<div className="flex items-center gap-2">{rightNodes}</div>
			</div>
		</div>
	);
};
