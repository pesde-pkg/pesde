import starlight from "@astrojs/starlight"
import tailwind from "@astrojs/tailwind"
import { defineConfig } from "astro/config"

import vercel from "@astrojs/vercel/serverless"

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
					items: [
						{
							label: "Getting Started",
							slug: "guides/getting-started",
						},
					],
				},
				{
					label: "Reference",
					autogenerate: {
						directory: "reference",
					},
				},
			],
			components: {
				SiteTitle: "./src/components/SiteTitle.astro",
			},
			customCss: ["./src/tailwind.css", "@fontsource-variable/nunito-sans"],
			favicon: "/favicon.ico",
			head: [
				{
					tag: "meta",
					attrs: {
						name: "theme-color",
						content: "#F19D1E",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "icon",
						type: "image/png",
						href: "/favicon-48x48.png",
						sizes: "48x48",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "icon",
						type: "image/svg+xml",
						href: "/favicon.svg",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "apple-touch-icon",
						sizes: "180x180",
						href: "/apple-touch-icon.png",
					},
				},
				{
					tag: "meta",
					attrs: {
						name: "apple-mobile-web-app-title",
						content: "pesde docs",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "manifest",
						href: "/site.webmanifest",
					},
				},
			],
		}),
		tailwind({
			applyBaseStyles: false,
		}),
	],
	vite: {
		envDir: "..",
	},
	output: "hybrid",
	adapter: vercel(),
})
