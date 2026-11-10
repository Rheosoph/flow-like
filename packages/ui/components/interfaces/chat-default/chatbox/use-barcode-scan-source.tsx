"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	ComposerAttachResult,
	ComposerAttachSource,
} from "../../../../state/composer-capture";
import { type BarcodeDetect, barcodeMessage } from "./barcode-frames";
import {
	type BarcodeCapture,
	BarcodeScannerDialog,
} from "./barcode-scanner-dialog";

export interface BarcodeScanSource {
	readonly source: ComposerAttachSource | undefined;
	readonly dialog: ReactNode;
}

function cameraAvailable(): boolean {
	return (
		typeof navigator !== "undefined" &&
		typeof navigator.mediaDevices?.getUserMedia === "function"
	);
}

/** "Scan barcodes" for hosts that can read codes; render `dialog` next to the chats. */
export function useBarcodeScanSource(
	detect: BarcodeDetect | undefined,
): BarcodeScanSource {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const resolver = useRef<((capture: BarcodeCapture | null) => void) | null>(
		null,
	);

	const settle = useCallback((capture: BarcodeCapture | null) => {
		resolver.current?.(capture);
		resolver.current = null;
		setOpen(false);
	}, []);

	useEffect(() => () => settle(null), [settle]);

	const attach = useCallback(async (): Promise<ComposerAttachResult> => {
		resolver.current?.(null);
		const capture = await new Promise<BarcodeCapture | null>((resolve) => {
			resolver.current = resolve;
			setOpen(true);
		});
		if (!capture) return {};
		return {
			files: [capture.photo],
			text: barcodeMessage(
				capture.barcodes,
				t("scannedBarcodesHeading", "Scanned barcodes:"),
			),
		};
	}, [t]);

	const source = useMemo<ComposerAttachSource | undefined>(
		() =>
			detect && cameraAvailable() ? { kind: "barcode", attach } : undefined,
		[detect, attach],
	);
	const dialog = detect ? (
		<BarcodeScannerDialog
			open={open}
			detect={detect}
			onCapture={settle}
			onCancel={() => settle(null)}
		/>
	) : null;
	return { source, dialog };
}
