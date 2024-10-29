<script module lang="ts">
	import type { Action } from "svelte/action"

	export const tocScroll: Action<HTMLElement> = (node) => {
		$effect(() => {
			const link = node.querySelector(`[data-item-id="${activeHeaderId.value}"]`)

			if (link && link instanceof HTMLElement) {
				setTimeout(() => {
					node.scrollTo({
						top: link.offsetTop + link.clientHeight - node.clientHeight / 2,
						behavior: "smooth",
					})
				}, 20)
			}
		})
	}
</script>

<script lang="ts">
	import type { TocItem } from "$lib/markdown"
	import { activeHeaderId } from "./TocObserver.svelte"

	type Props = {
		toc: TocItem[]
		mobile?: boolean
	}

	const { toc, mobile = false }: Props = $props()
</script>

<ul>
	{#each toc as item}
		{@const active = item.id === activeHeaderId.value}
		<li data-item-id={item.id}>
			<a
				href={`#${item.id}`}
				class={`-ml-px block truncate pl-[calc(var(--level)*theme(spacing.4))] transition ${active ? "text-primary" : "hover:text-heading"} ${mobile ? "py-2" : "py-1"}`}
				style:--level={item.level - 1}
			>
				{item.title}
			</a>
		</li>
	{/each}
</ul>
