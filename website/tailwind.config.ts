import type { Config } from 'tailwindcss';

export default {
	content: ['./src/**/*.{html,js,svelte,ts}'],

	theme: {
		extend: {}
	},

	// eslint-disable-next-line @typescript-eslint/no-require-imports
	plugins: [require('@tailwindcss/typography')]
} as Config;
