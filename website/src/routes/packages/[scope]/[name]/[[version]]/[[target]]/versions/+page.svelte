<script lang="ts">
	import { formatDistanceToNow } from "date-fns"

	const { data } = $props()
</script>

<div class="space-y-4 py-4">
	{#each data.versions as pkg}
		<article
			class="relative overflow-hidden rounded bg-card px-5 py-4 transition hover:bg-card-hover"
		>
			<h2 class="font-semibold text-heading">
				<a
					href={`/packages/${pkg.name}/${pkg.version}/any`}
					class="after:absolute after:inset-0 after:content-['']"
				>
					{pkg.version}
				</a>
			</h2>
			<div class="text-sm font-semibold">
				<time>{formatDistanceToNow(new Date(pkg.published_at), { addSuffix: true })}</time>
				·
				{pkg.targets
					.map((target) => {
						return target.kind[0].toUpperCase() + target.kind.slice(1)
					})
					.join(", ")}
			</div>
		</article>
	{/each}
</div>
