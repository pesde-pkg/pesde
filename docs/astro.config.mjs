import { defineConfig } from "astro/config"
import starlight from "@astrojs/starlight"
import tailwind from "@astrojs/tailwind"

// https://astro.build/config
export default defineConfig({
	redirects: {
		"/": "/guides/getting-started",
	},
	integrations: [
		starlight({
			title: "pesde docs",
			social: {
				github: "https://github.com/daimond113/pesde",
			},
			sidebar: [
				{
					label: "Guides",
					items: [{ label: "Getting Started", slug: "guides/getting-started" }],
				},
				{
					label: "Reference",
					autogenerate: { directory: "reference" },
				},
			],
			components: {
				SiteTitle: "./src/components/SiteTitle.astro",
			},
			customCss: ["./src/tailwind.css"],
		}),
		tailwind({ applyBaseStyles: false }),
	],
	vite: {
		envDir: "..",
	},
})
