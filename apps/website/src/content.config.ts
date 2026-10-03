import { defineCollection, z } from "astro:content";
import { glob } from "astro/loaders";

const blog = defineCollection({
	loader: glob({ pattern: "**/*.{md,mdx}", base: "./src/content/blog" }),
	schema: z.object({
		title: z.string(),
		description: z.string().max(200).optional(),
		date: z.coerce.date(), // accepts string dates
		updated: z.coerce.date().optional(),
		draft: z.boolean().default(false),
		// Tags become URL segments, so casing must not create duplicate routes.
		tags: z
			.array(z.string().trim().toLowerCase())
			.default([])
			.transform((tags) => [...new Set(tags)]),
		cover: z.string().optional(), // /public/... or remote URL
		canonical: z.string().url().optional(),
	}),
});

export const collections = { blog };
