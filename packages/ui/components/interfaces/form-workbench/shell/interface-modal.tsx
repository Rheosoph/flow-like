"use client";

import {
	type KeyboardEvent as ReactKeyboardEvent,
	type RefObject,
	createContext,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import { cx } from "../../../settings/devices/primitives/tone";
import type { InterfaceModalProps } from "../contracts";
import {
	type FocusSpot,
	focusOnPage,
	focusSpotOf,
	returnFocusTo,
	selectContents,
	useIsoLayoutEffect,
} from "./focus";
import { isComposingKey } from "./keyboard";

/**
 * Where modals render: an element at the end of the interface root, so a scrim covers the interface and
 * never the host's header. Without a shell around (a view on its own) a modal renders in place.
 */
export const ModalLayerContext = createContext<HTMLElement | null>(null);

const LAYER_CLASS = "pointer-events-none absolute inset-0 z-40";

/** Creates the modal layer once and keeps it as the last child of the interface root. */
export function useModalLayer(rootRef: RefObject<HTMLElement | null>) {
	const [layer] = useState<HTMLElement | null>(() => {
		if (typeof document === "undefined") return null;
		const element = document.createElement("div");
		element.className = LAYER_CLASS;
		element.setAttribute("data-fw-modal-layer", "");
		return element;
	});
	useIsoLayoutEffect(() => {
		const root = rootRef.current;
		if (!root || !layer) return;
		root.appendChild(layer);
		return () => layer.remove();
	}, [rootRef, layer]);
	return layer;
}

const TABBABLE =
	'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

const tabbablesOf = (container: HTMLElement) =>
	Array.from(container.querySelectorAll<HTMLElement>(TABBABLE));

/**
 * Where focus was before the dialog opened. Read in a layout effect: a control inside the dialog may take
 * focus from its own effect before the dialog's effect runs.
 */
function useOpener(open: boolean, dialogRef: RefObject<HTMLElement | null>) {
	const opener = useRef<FocusSpot | null>(null);
	useIsoLayoutEffect(() => {
		const dialog = dialogRef.current;
		if (!open || !dialog) return;
		const spot = focusSpotOf(dialog.ownerDocument);
		opener.current = spot && !dialog.contains(spot.element) ? spot : null;
	}, [open, dialogRef]);
	return opener;
}

/** Focus goes into the dialog when it opens (`data-autofocus`, else the first control) and back out when it closes. */
function useModalFocus(
	open: boolean,
	dialogRef: RefObject<HTMLElement | null>,
) {
	const opener = useOpener(open, dialogRef);
	useEffect(() => {
		const dialog = dialogRef.current;
		if (!open || !dialog) return;
		const from = opener.current;
		const root = dialog.closest<HTMLElement>("[data-fw-root]");
		const wanted = dialog.querySelector<HTMLElement>("[data-autofocus]");
		const first = wanted ?? tabbablesOf(dialog)[0] ?? dialog;
		first.focus();
		if (wanted) selectContents(wanted);
		return () => {
			if (!focusOnPage(dialog.ownerDocument) || returnFocusTo(from)) return;
			root?.focus({ preventScroll: true });
		};
	}, [open, dialogRef, opener]);
}

function keepTabInside(
	event: ReactKeyboardEvent<HTMLElement>,
	dialog: HTMLElement,
) {
	const items = tabbablesOf(dialog);
	const first = items[0];
	const last = items[items.length - 1];
	const active = dialog.ownerDocument.activeElement;
	if (!first || !last) {
		event.preventDefault();
		dialog.focus();
	} else if (event.shiftKey && (active === first || active === dialog)) {
		event.preventDefault();
		last.focus();
	} else if (!event.shiftKey && active === last) {
		event.preventDefault();
		first.focus();
	}
}

export interface InterfaceModalFrameProps extends InterfaceModalProps {
	/** `top`: 72 px below the top of the interface (the save dialog). `center`: centred (leave, shortcuts). */
	readonly placement?: "top" | "center";
}

/**
 * A modal inside the interface box: the scrim covers the interface only, focus is trapped, Esc and a
 * click on the scrim close it, focus returns to where it was. `width` is a ceiling: the frame never
 * outgrows the box. Put `data-autofocus` on the control that should take focus first.
 */
export function InterfaceModal({
	open,
	onClose,
	labelledBy,
	width,
	children,
	placement = "top",
}: Readonly<InterfaceModalFrameProps>) {
	const layer = useContext(ModalLayerContext);
	const dialogRef = useRef<HTMLDivElement>(null);
	useModalFocus(open, dialogRef);
	if (!open) return null;

	const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
		if (event.defaultPrevented || isComposingKey(event)) return;
		if (event.key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			onClose();
		} else if (event.key === "Tab" && dialogRef.current) {
			keepTabInside(event, dialogRef.current);
		}
	};

	const frame = (
		<div
			data-fw-modal-frame=""
			className={cx(
				"absolute inset-0 flex justify-center px-4",
				placement === "center"
					? "items-center py-8"
					: "items-start pt-[72px] pb-6",
			)}
		>
			<button
				type="button"
				tabIndex={-1}
				aria-hidden="true"
				data-fw-scrim=""
				className="pointer-events-auto absolute inset-0 cursor-default bg-scrim"
				onClick={onClose}
			/>
			<div
				ref={dialogRef}
				// biome-ignore lint/a11y/useSemanticElements: a native <dialog> brings its own top-layer and positioning, which the interface-box frame and the shared focus trap replace
				role="dialog"
				aria-modal="true"
				aria-labelledby={labelledBy}
				tabIndex={-1}
				data-fw-modal=""
				data-placement={placement}
				onKeyDown={onKeyDown}
				style={{ width }}
				className="pointer-events-auto relative flex max-h-full max-w-full flex-col overflow-y-auto rounded-[10px] border border-border-strong bg-popover text-popover-foreground shadow-floating outline-none"
			>
				{children}
			</div>
		</div>
	);
	return layer ? createPortal(frame, layer) : frame;
}
