import { setWorkerUrl } from "maplibre-gl";

export function configureMapLibreWorker() {
	const workerUrl = process.env.NEXT_PUBLIC_MAPLIBRE_WORKER_URL;
	if (!workerUrl)
		throw new Error("MapLibre worker assets were not configured by the build.");
	setWorkerUrl(workerUrl);
}
