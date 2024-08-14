import { fetchRegistryJson, type SearchResponse } from "$lib/registry-api"
import type { PageServerLoad } from "./$types"

export const load: PageServerLoad = async ({ fetch }) => {
	const { data: packages } = await fetchRegistryJson<SearchResponse>("search", fetch)

	return { packages }
}
