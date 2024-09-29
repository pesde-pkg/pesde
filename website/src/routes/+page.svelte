<script lang="ts">
	import { formatDistanceToNow } from "date-fns"

	import { TARGET_KIND_DISPLAY_NAMES } from "$lib/registry-api"
	import Hero from "./Hero.svelte"

	const { data } = $props()

	let displayDates = $state(false)

	$effect(() => {
		displayDates = true
	})
</script>

<Hero />

<section class="mx-auto max-w-screen-lg px-4 pb-32">
	<h2 class="text-heading mb-4 text-2xl font-semibold">
		<a id="recently-published" href="#recently-published">Recently Published</a>
	</h2>

	<div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
		{#each data.packages.slice(0, 24) as pkg}
			{@const [scope, name] = pkg.name.split("/")}

			<article
				class="bg-card hover:bg-card-hover relative overflow-hidden rounded px-5 py-4 transition"
			>
				<h3 class="truncate text-xl font-semibold">
					<a href={`/packages/${pkg.name}`} class="after:absolute after:inset-0 after:content-['']">
						<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
					</a>
				</h3>
				<div class="text-primary mb-3 flex overflow-hidden whitespace-nowrap text-sm font-semibold">
					<span class="truncate">v{pkg.version}</span>
					<span class="whitespace-pre"
						>{` · ${pkg.targets
							.map((target) => TARGET_KIND_DISPLAY_NAMES[target.kind])
							.join(", ")}`}</span
					>
				</div>
				<p class="mb-3 line-clamp-2 h-[2lh] overflow-hidden text-sm">{pkg.description}</p>
				<div class="text-heading text-sm font-semibold">
					<time datetime={pkg.published_at} class:invisible={!displayDates}>
						{#if displayDates}
							{formatDistanceToNow(new Date(pkg.published_at), { addSuffix: true })}
						{:else}
							...
						{/if}
					</time>
				</div>
			</article>
		{/each}
	</div>
</section>
