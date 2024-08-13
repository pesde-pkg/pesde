<script lang="ts">
	import type { PageData } from "./$types"
	import { formatDistanceToNow } from "date-fns"

	import Hero from "./Hero.svelte"

	export let data: PageData
</script>

<Hero />

<section class="mx-auto max-w-screen-xl px-4 pb-32">
	<h2 class="mb-4 text-2xl font-semibold text-heading">
		<a id="recently-published" href="#recently-published">Recently Published</a>
	</h2>

	<div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
		{#each data.packages.slice(0, 24) as pkg}
			{@const [scope, name] = pkg.name.split("/")}

			<article
				class="relative overflow-hidden rounded bg-card px-5 py-4 transition hover:bg-card-hover"
			>
				<h3 class="truncate text-xl font-semibold">
					<a href={`/packages/${pkg.name}`} class="after:absolute after:inset-0 after:content-['']">
						<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
					</a>
				</h3>
				<div class="mb-3 flex overflow-hidden whitespace-nowrap text-sm font-semibold text-primary">
					<span class="truncate">v{pkg.version}</span>
					<span class="whitespace-pre"
						>{` · ${pkg.targets
							.map((target) => {
								return target.kind[0].toUpperCase() + target.kind.slice(1)
							})
							.join(", ")}`}</span
					>
				</div>
				<p class="mb-3 line-clamp-2 h-[2lh] overflow-hidden text-sm">{pkg.description}</p>
				<div class="text-sm font-semibold text-heading">
					<time datetime={pkg.published_at}>
						{formatDistanceToNow(new Date(pkg.published_at), { addSuffix: true })}
					</time>
				</div>
			</article>
		{/each}
	</div>
</section>
