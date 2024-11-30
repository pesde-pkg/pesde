import adapter from "@sveltejs/adapter-cloudflare"
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte"
import { mdsvex } from "mdsvex"

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: [vitePreprocess(), mdsvex()],

	kit: {
		adapter: adapter({}),
		env: {
			dir: "..",
		},
	},

	extensions: [".svelte", ".svx"],
}

export default config
