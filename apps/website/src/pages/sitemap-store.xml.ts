import type { APIRoute } from "astro";
import { searchPackages, searchApps, isStoreEnabled } from "../lib/registry";

export const prerender = false;

export const GET: APIRoute = async () => {
	if (!isStoreEnabled()) {
		return new Response(
			'<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>',
			{ headers: { "Content-Type": "application/xml" } },
		);
	}

	const site = "https://flow-like.com";
	const now = new Date().toISOString();

	let urls: { loc: string; lastmod: string; priority: string; changefreq: string }[] = [];

	try {
		const [pkgRes, apps] = await Promise.all([
			searchPackages(500),
			searchApps(500),
		]);

		for (const pkg of pkgRes.packages) {
			urls.push({
				loc: `${site}/store/packages/${pkg.id}`,
				lastmod: now,
				priority: pkg.verified ? "0.8" : "0.6",
				changefreq: "weekly",
			});
		}

		for (const [app] of apps) {
			urls.push({
				loc: `${site}/store/apps/${app.id}`,
				lastmod: app.updated_at || now,
				priority: "0.8",
				changefreq: "weekly",
			});
		}
	} catch (e) {
		console.error("Sitemap store generation error:", e);
	}

	const xml = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls
	.map(
		(u) => `  <url>
    <loc>${u.loc}</loc>
    <lastmod>${u.lastmod}</lastmod>
    <changefreq>${u.changefreq}</changefreq>
    <priority>${u.priority}</priority>
  </url>`,
	)
	.join("\n")}
</urlset>`;

	return new Response(xml, {
		headers: {
			"Content-Type": "application/xml",
			"Cache-Control": "public, max-age=3600, s-maxage=3600",
		},
	});
};
