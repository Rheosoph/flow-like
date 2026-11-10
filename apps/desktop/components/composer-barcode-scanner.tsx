"use client";

import type {
	BarcodeDetect,
	DetectedBarcode,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox/barcode-frames";
import { useBarcodeScanSource } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox/use-barcode-scan-source";
import { useComposerCaptureStore } from "@flow-like/flow-like-ui/state/composer-capture";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useMemo } from "react";

/** Reads a frame with the Read Barcodes node's decoder (rxing) on this device. */
export const detectBarcodes: BarcodeDetect = async (frame, thorough) => {
	const codes = await invoke<DetectedBarcode[]>("detect_barcodes", frame.luma, {
		headers: {
			"x-frame-width": String(frame.width),
			"x-frame-height": String(frame.height),
			...(thorough ? { "x-frame-thorough": "1" } : {}),
		},
	});
	return codes.map(({ text, format, points }) => ({ text, format, points }));
};

/** Offers "Scan barcodes" in every chat composer of the desktop and mobile apps. */
export function ComposerBarcodeScanner() {
	const detect = useMemo(() => (isTauri() ? detectBarcodes : undefined), []);
	const { source, dialog } = useBarcodeScanSource(detect);
	useEffect(() => {
		if (!source) return;
		return useComposerCaptureStore.getState().register([source]);
	}, [source]);
	return dialog;
}
