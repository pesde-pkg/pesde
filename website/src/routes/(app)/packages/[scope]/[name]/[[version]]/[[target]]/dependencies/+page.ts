import type { PageLoad } from "./$types"

export const load: PageLoad = async ({ parent }) => {
	const data = await parent()

	return {
		meta: {
			title: `${data.pkg.name} - ${data.pkg.version} - dependencies`,
			description: data.pkg.description,
		},
	}
}
