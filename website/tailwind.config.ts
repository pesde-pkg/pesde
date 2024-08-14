import type { Config } from "tailwindcss"
import defaultTheme from "tailwindcss/defaultTheme"

const alpha = (color: string, alpha: number = 1) => color.replace("<alpha-value>", alpha.toString())

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
				border: "rgb(var(--color-border) / <alpha-value>)",

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
					bg: "rgb(var(--color-primary-bg) / <alpha-value>)",
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
			borderColor: {
				DEFAULT: "rgb(var(--color-border) / <alpha-value>)",
			},
			typography: ({ theme }) => ({
				DEFAULT: {
					css: {
						"--tw-prose-body": alpha(theme("colors.body")),
						"--tw-prose-headings": alpha(theme("colors.heading")),
						"--tw-prose-lead": alpha(theme("colors.heading")),
						"--tw-prose-links": alpha(theme("colors.primary").DEFAULT),
						"--tw-prose-bold": alpha(theme("colors.body")),
						"--tw-prose-counters": alpha(theme("colors.body")),
						"--tw-prose-bullets": alpha(theme("colors.border")),
						"--tw-prose-hr": alpha(theme("colors.border")),
						"--tw-prose-quotes": alpha(theme("colors.body")),
						"--tw-prose-quote-borders": alpha(theme("colors.border")),
						"--tw-prose-captions": alpha(theme("colors.body")),
						"--tw-prose-code": alpha(theme("colors.body")),
						"--tw-prose-pre-code": alpha(theme("colors.body")),
						"--tw-prose-pre-bg": alpha(theme("colors.card").DEFAULT),
						"--tw-prose-th-borders": alpha(theme("colors.border")),
						"--tw-prose-td-borders": alpha(theme("colors.border")),
					},
				},
			}),
		},
	},

	// eslint-disable-next-line @typescript-eslint/no-require-imports
	plugins: [require("@tailwindcss/typography")],
} as Config
