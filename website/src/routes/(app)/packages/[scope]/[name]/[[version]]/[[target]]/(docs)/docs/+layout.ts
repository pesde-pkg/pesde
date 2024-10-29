import type { DocEntry } from "$lib/registry-api"
import type { SidebarItem } from "../Sidebar.svelte"
import type { LayoutLoad } from "./$types"

export const load: LayoutLoad = async ({ params, parent }) => {
	const parentData = await parent()

	const { scope, name, version, target } = params
	const basePath = `/packages/${scope}/${name}/${version ?? "latest"}/${target ?? "any"}`

	function docEntryToSidebarItem(entry: DocEntry): SidebarItem {
		if ("name" in entry) {
			return {
				label: entry.label,
				href: `${basePath}/docs/${entry.name}`,
			}
		}

		return {
			label: entry.label,
			children: entry.items?.map(docEntryToSidebarItem) ?? [],
			collapsed: entry.collapsed ?? false,
		}
	}

	const sidebar = parentData.pkg.docs?.map(docEntryToSidebarItem) ?? []

	return {
		activeTab: "docs",
		doc: params.doc,
		sidebar,
	}
}
