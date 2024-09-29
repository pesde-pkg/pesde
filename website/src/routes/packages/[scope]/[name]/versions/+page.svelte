<script lang="ts">
	import { TARGET_KIND_DISPLAY_NAMES } from "$lib/registry-api.js"
	import { formatDistanceToNow } from "date-fns"

	const { data } = $props()

	let displayDates = $state(false)
	$effect(() => {
		displayDates = true
	})
</script>

<div class="space-y-4 py-4">
	{#each data.versions as pkg, index}
		{@const isLatest = index === 0}

		<article
			class={`bg-card hover:bg-card-hover relative overflow-hidden rounded px-5 py-4 transition ${
				isLatest ? "ring-primary ring-2 ring-inset" : ""
			}`}
		>
			<h2 class="text-heading font-semibold">
				<a
					href={`/packages/${pkg.name}/${pkg.version}/any`}
					class="after:absolute after:inset-0 after:content-['']"
				>
					{pkg.version}
					{#if isLatest}
						<span class="text-primary">(latest)</span>
					{/if}
				</a>
			</h2>
			<div class="text-sm font-semibold" class:invisible={!displayDates}>
				<time datetime={pkg.published_at}>
					{#if displayDates}
						{formatDistanceToNow(new Date(pkg.published_at), { addSuffix: true })}
					{:else}
						...
					{/if}
				</time>
				·
				{pkg.targets.map((target) => TARGET_KIND_DISPLAY_NAMES[target.kind]).join(", ")}
			</div>
		</article>
	{/each}
</div>
