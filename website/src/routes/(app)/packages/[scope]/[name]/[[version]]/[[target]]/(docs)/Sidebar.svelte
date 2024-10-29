<script module lang="ts">
	export type SidebarItem =
		| {
				label: string
				href: string
		  }
		| {
				label: string
				children: SidebarItem[]
				collapsed: boolean
		  }

	// this comment is here to fix the syntax highlighting
</script>

<script lang="ts">
	import SidebarItemComponent from "./SidebarItem.svelte"

	type Props = {
		items: SidebarItem[]
	}

	const { items }: Props = $props()
</script>

<nav
	class="sticky top-14 -mt-12 hidden h-[calc(100vh-theme(spacing.14))] w-72 flex-shrink-0 flex-col overflow-y-auto border-r p-4 lg:flex xl:mt-0"
>
	<ul>
		{#each items as item}
			<li>
				<SidebarItemComponent {item} />
			</li>
		{/each}
	</ul>
</nav>
