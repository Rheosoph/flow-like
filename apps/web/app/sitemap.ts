import type { MetadataRoute } from "next";
import { PUBLIC_ROUTES } from "../lib/public-routes";

export const dynamic = "force-static";

const siteUrl = process.env.NEXT_PUBLIC_SITE_URL || "https://app.flow-like.com";

export default function sitemap(): MetadataRoute.Sitemap {
	const lastModified = new Date();

	return PUBLIC_ROUTES.map(({ path }) => ({
		url: `${siteUrl}${path}`,
		lastModified,
		changeFrequency: path === "/" ? "daily" : "weekly",
		priority: path === "/" ? 1 : 0.7,
	}));
}
