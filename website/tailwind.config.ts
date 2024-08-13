import type { Config } from "tailwindcss"
import defaultTheme from "tailwindcss/defaultTheme"

export default {
	content: ["./src/**/*.{html,js,svelte,ts}"],

	theme: {
		extend: {
			fontFamily: {
				sans: ["Nunito Sans Variable", ...defaultTheme.fontFamily.sans],
			},
			colors: {
				background: "rgb(var(--color-background) / <alpha-value>)",
				card: {
					DEFAULT: "rgb(var(--color-card) / <alpha-value>)",
					hover: "rgb(var(--color-card-hover) / <alpha-value>)",
				},

				body: "rgb(var(--color-body) / <alpha-value>)",
				heading: "rgb(var(--color-heading) / <alpha-value>)",
				light: "rgb(var(--color-light) / <alpha-value>)",

				input: {
					bg: "rgb(var(--color-input-bg) / <alpha-value>)",
					border: "rgb(var(--color-input-border) / <alpha-value>)",
				},
				placeholder: "rgb(var(--color-placeholder) / <alpha-value>)",

				primary: {
					DEFAULT: "rgb(var(--color-primary) / <alpha-value>)",
					hover: "rgb(var(--color-primary-hover) / <alpha-value>)",
					fg: "rgb(var(--color-primary-fg) / <alpha-value>)",
				},
			},
			animation: {
				"cursor-blink": "cursor-blink 1s ease-in-out 500ms infinite",
			},
			borderRadius: {
				none: "0",
				sm: `${4 / 16}rem`,
				DEFAULT: `${8 / 16}rem`,
			},
		},
	},

	// eslint-disable-next-line @typescript-eslint/no-require-imports
	plugins: [require("@tailwindcss/typography")],
} as Config
