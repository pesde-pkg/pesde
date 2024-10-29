import type { LayoutLoad } from "./$types"

export const load: LayoutLoad = async ({ params, parent }) => {
	const { scope, name, version, target } = params
	const basePath = `/packages/${scope}/${name}/${version ?? "latest"}/${target ?? "any"}`

	const parentData = await parent()
	return {
		activeTab: "reference",
		sidebar: [
			{
				label: "Reference",
				href: `${basePath}/reference`,
			},
		],
		toc: [
			{
				id: "_top",
				title: "Overview",
				level: 2,
			},
		],
		meta: {
			siteName: `${parentData.pkg.name} - pesde`,
			title: "Reference",
			description: `API reference for ${parentData.pkg.name}`,
		},
	}
}
