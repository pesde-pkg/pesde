<script lang="ts">
	import { page } from "$app/stores"
	import type { Snippet } from "svelte"

	type Props = {
		tab: string
		children: Snippet
	}

	const { tab, children }: Props = $props()

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		return `/packages/${encodeURIComponent(scope)}/${encodeURIComponent(name)}`
	})

	const activeTab = $derived(
		$page.url.pathname.slice(basePath.length).replace(/^\//, "").replace(/\/$/, ""),
	)

	const href = $derived(`${basePath}/${tab}`)
	const active = $derived(activeTab === tab)

	const linkClass = $derived(
		"font-semibold px-5 inline-flex items-center transition rounded-r h-12 border-l-2 " +
			"sm:rounded-r-none sm:rounded-t sm:border-b-2 sm:border-l-0 sm:h-10 sm:-mb-0.5 " +
			(active ? "text-primary border-primary bg-primary-bg/20" : "hover:bg-border/30"),
	)
</script>

<a {href} class={linkClass}>
	{@render children()}
</a>
