<script lang="ts">
	import { page } from "$app/stores"
	import type { Snippet } from "svelte"
	import type { PageData } from "./$types"

	type Props = {
		tab: string
		children: Snippet
	}

	const { tab, children }: Props = $props()
	const pkg = $derived(($page.data as PageData).pkg)

	const shortBasePath = $derived(`/packages/${pkg.name}`)
	const fullBasePath = $derived(`${shortBasePath}/${pkg.version}/${pkg.targets[0].kind}`)

	const isFullPath = $derived($page.url.pathname.startsWith(fullBasePath))
	const basePath = $derived(isFullPath ? fullBasePath : shortBasePath)

	const activeTab = $derived(
		$page.url.pathname.slice(basePath.length).replace(/^\//, "").replace(/\/$/, ""),
	)

	const href = $derived(`${basePath}/${tab}`)
	const active = $derived(activeTab === tab)

	const linkClass = $derived(
		`font-semibold px-5 h-10 inline-flex -mb-0.5 items-center rounded-t border-b-2 transition ${active ? "text-primary border-b-primary bg-primary-bg/20" : "hover:bg-border/30"}`,
	)
</script>

<a {href} class={linkClass}>
	{@render children()}
</a>
