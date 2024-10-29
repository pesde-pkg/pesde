<script lang="ts">
	import type { TocItem } from "$lib/markdown"
	import { Popover } from "bits-ui"
	import { ChevronRight } from "lucide-svelte"
	import { scale } from "svelte/transition"
	import TocList, { tocScroll } from "./TocList.svelte"
	import { activeHeaderId } from "./TocObserver.svelte"

	type Props = {
		toc: TocItem[]
	}

	const { toc }: Props = $props()

	let activeHeaderTitle = $derived(toc.find((item) => item.id === activeHeaderId.value)?.title)
	let popoverOpen = $state(false)
</script>

<nav class="bg-header/80 sticky top-14 border-b backdrop-blur-lg lg:ml-72 xl:hidden">
	<div class="mx-auto flex h-12 max-w-screen-md items-center px-4 md:px-6">
		<Popover.Root bind:open={popoverOpen}>
			<Popover.Trigger
				class="bg-background/80 flex h-8 flex-shrink-0 items-center rounded border px-4 text-sm font-semibold {popoverOpen
					? 'border-primary'
					: 'hover:border-body/50'}"
			>
				On this page
				<ChevronRight class="ml-0.5 size-4" />
			</Popover.Trigger>
			<Popover.Content side="bottom" align="start" sideOffset={4} collisionPadding={8}>
				{#snippet child({ props })}
						<div
							class="bg-card border-input-border max-h-[var(--bits-popover-content-available-height)] w-72 max-w-[var(--bits-popover-content-available-width)] origin-top-left overflow-y-auto rounded border p-4 pl-2 pr-6 shadow-lg"
							use:tocScroll
							transition:scale={{ start: 0.95, duration: 200 }}
							{...props}
						>
							<TocList mobile {toc} />
						</div>
				{/snippet}
			</Popover.Content>
		</Popover.Root>

		{#if activeHeaderTitle}
			<span class="text-heading ml-2 truncate text-sm">{activeHeaderTitle}</span>
		{/if}
	</div>
</nav>
