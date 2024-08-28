import type { Config } from "tailwindcss"
import starlightPlugin from "@astrojs/starlight-tailwind"
import defaultTheme from "tailwindcss/defaultTheme"

export default {
	content: ["./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}"],

	theme: {
		extend: {
			fontFamily: {
				sans: ["Nunito Sans Variable", ...defaultTheme.fontFamily.sans],
			},
			colors: {
				accent: {
					200: "rgb(241 157 30)",
					600: "rgb(120 70 10)",
					900: "rgb(24 16 8)",
					950: "rgb(10 7 4)",
				},
				gray: {
					100: "rgb(245 230 210)",
					200: "rgb(228 212 192)",
					300: "rgb(180 160 140)",
					400: "rgb(130 90 40)",
					500: "rgb(84 70 50)",
					600: "rgb(65 50 41)",
					700: "rgb(50 42 35)",
					800: "rgb(28 22 17)",
					900: "rgb(10 7 4)",
				},
			},
		},
	},

	plugins: [starlightPlugin()],
} as Config
