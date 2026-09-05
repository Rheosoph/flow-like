import { captureServerRequestError } from "@/lib/server-telemetry";
import type { Instrumentation } from "next";

export const onRequestError: Instrumentation.onRequestError = (
	error,
	request,
	context,
) => {
	captureServerRequestError(error, request, context);
};
