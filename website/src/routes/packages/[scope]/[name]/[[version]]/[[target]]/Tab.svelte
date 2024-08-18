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
		if ("target" in $page.params) {
			const { version, target } = $page.params
			return `/packages/${scope}/${name}/${version}/${target}`
		}
		return `/packages/${scope}/${name}`
	})

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
