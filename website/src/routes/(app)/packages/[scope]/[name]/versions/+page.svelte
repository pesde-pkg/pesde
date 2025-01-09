<script lang="ts">
	import { TARGET_KIND_DISPLAY_NAMES, type TargetKind } from "$lib/registry-api.js"
	import { formatDistanceToNow } from "date-fns"

	const { data } = $props()

	let displayDates = $state(false)
	$effect(() => {
		displayDates = true
	})
</script>

<div class="space-y-4 py-4">
	{#each data.versions as pkgVersion, index}
		{@const isLatest = index === 0}

		<article
			class={`bg-card hover:bg-card-hover relative overflow-hidden rounded px-5 py-4 transition ${
				isLatest ? "ring-primary ring-2 ring-inset" : ""
			}`}
			class:opacity-50={Object.values(pkgVersion.targets).every(({ yanked }) => yanked)}
		>
			<h2 class="text-heading font-semibold">
				<a
					href={`/packages/${data.name}/${pkgVersion.version}/any`}
					class="after:absolute after:inset-0 after:content-['']"
				>
					{pkgVersion.version}
					{#if isLatest}
						<span class="text-primary">(latest)</span>
					{/if}
				</a>
			</h2>
			<div class="text-sm font-semibold" class:invisible={!displayDates}>
				<time datetime={pkgVersion.published_at}>
					{#if displayDates}
						{formatDistanceToNow(new Date(pkgVersion.published_at), { addSuffix: true })}
					{:else}
						...
					{/if}
				</time>
				·
				{#each Object.entries(pkgVersion.targets) as [target, info], index}
					{#if index > 0}
						<span>, </span>
					{/if}
					<span class:line-through={info.yanked}
						>{TARGET_KIND_DISPLAY_NAMES[target as TargetKind]}</span
					>
				{/each}
			</div>
		</article>
	{/each}
</div>
