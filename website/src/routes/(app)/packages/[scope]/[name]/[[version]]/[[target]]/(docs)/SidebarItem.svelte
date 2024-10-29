<script lang="ts">
	import { page } from "$app/stores"
	import { ChevronDownIcon } from "lucide-svelte"
	import type { SidebarItem } from "./Sidebar.svelte"
	import Self from "./SidebarItem.svelte"

	type Props = {
		item: SidebarItem
	}

	const { item }: Props = $props()

	let open = $state("collapsed" in item ? !item.collapsed : true)

	let active = $derived.by(() => {
		if ("href" in item) {
			const fullUrl = new URL(item.href, $page.url)
			return fullUrl.pathname === $page.url.pathname
		}
		return false
	})
</script>

{#if "href" in item}
	<a
		href={item.href}
		class={`-mx-2 block rounded px-2 py-1.5 text-sm transition ${active ? "bg-primary-bg/20 text-primary font-semibold" : "hover:text-heading"}`}
	>
		{item.label}
	</a>
{:else}
	<details class="group flex flex-col py-0.5" bind:open>
		<summary class="text-heading flex list-none items-center py-1 font-bold">
			<span>{item.label} </span>
			<ChevronDownIcon class="ml-auto size-5 transition group-[:not([open])]:-rotate-90" />
		</summary>
		<ul class="mb-1 border-l pl-4">
			{#each item.children as child}
				<li>
					<Self item={child} />
				</li>
			{/each}
		</ul>
	</details>
{/if}
