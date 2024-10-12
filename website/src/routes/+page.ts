import { fetchRegistryJson, type SearchResponse } from "$lib/registry-api"
import type { PageLoad } from "./$types"

export const load: PageLoad = async ({ fetch }) => {
	const { data: packages } = await fetchRegistryJson<SearchResponse>("search", fetch)

	return {
		packages,
	}
}
