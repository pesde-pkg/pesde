import { fetchRegistry, type PackageResponse } from "$lib/registry-api"
import type { PageServerLoad } from "./$types"

export const load: PageServerLoad = async ({ fetch }) => {
	const packages = await fetchRegistry<PackageResponse[]>("search", fetch)

	return { packages }
}
