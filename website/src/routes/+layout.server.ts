import { ISR_BYPASS_TOKEN } from "$env/static/private"
import type { Config } from "@sveltejs/adapter-vercel"

export const config: Config = {
	isr: {
		expiration: 30 * 60,
		bypassToken: ISR_BYPASS_TOKEN,
	},
}
