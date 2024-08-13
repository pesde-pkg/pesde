import { fetchRegistry, type SearchResponse } from "$lib/registry-api"
import type { PageServerLoad } from "./$types"

export const load: PageServerLoad = async ({ fetch }) => {
	const { data: packages } = await fetchRegistry<SearchResponse>("search", fetch)

	return { packages }
}
