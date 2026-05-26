"use client";

import { NotFoundPage } from "@flow-like/flow-like-ui";

export default function NotFound() {
	return <NotFoundPage onGoBack={() => window.history.back()} homeHref="/" />;
}
