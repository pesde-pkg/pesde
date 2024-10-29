<script lang="ts">
	import { page } from "$app/stores"
	import type { Snippet } from "svelte"

	type Props = {
		tab: string
		children: Snippet
		active?: boolean
	}

	const { tab, children, active = false }: Props = $props()

	const basePath = $derived.by(() => {
		const { scope, name, version, target } = $page.params
		return `/packages/${scope}/${name}/${version ?? "latest"}/${target ?? "any"}`
	})

	const href = $derived(`${basePath}/${tab}`)
</script>

<a
	{href}
	class={`-mb-px flex h-9 w-full items-center justify-center border-b-2 px-4 font-semibold transition sm:w-auto ${active ? "border-primary text-heading" : "hover:border-border hover:text-heading border-transparent"}`}
>
	{@render children()}
</a>
