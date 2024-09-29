import { fetchRegistryJson, type SearchResponse } from "$lib/registry-api"
import type { PageLoad } from "./$types"

const PAGE_SIZE = 50

export const load: PageLoad = async ({ fetch, url }) => {
	const query = url.searchParams.get("q") ?? ""

	let page = parseInt(url.searchParams.get("page") ?? "1")
	if (isNaN(page) || page < 1) {
		page = 1
	}

	const params = new URLSearchParams()
	params.set("query", query)
	params.set("offset", String((page - 1) * PAGE_SIZE))

	const result = fetchRegistryJson<SearchResponse>(`search?${params}`, fetch)

	return {
		query,
		page,
		pageSize: PAGE_SIZE,
		result,
	}
}
