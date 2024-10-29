<script lang="ts">
	import type { TocItem } from "$lib/markdown"
	import TocList, { tocScroll } from "./TocList.svelte"

	type Props = {
		toc: TocItem[]
	}

	const { toc }: Props = $props()
</script>

<aside
	class="hide-scrollbar sticky top-14 hidden h-[calc(100vh-theme(spacing.14))] w-72 flex-shrink-0 overflow-y-auto py-16 text-lg xl:block"
	use:tocScroll
>
	<nav class="border-l pr-4 text-sm">
		<h2 class="text-heading mb-1 pl-4 text-base font-semibold">On this page</h2>
		<TocList {toc} />
	</nav>
</aside>
