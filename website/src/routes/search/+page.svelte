<script lang="ts">
	import { goto } from "$app/navigation"
	import { TARGET_KIND_DISPLAY_NAMES } from "$lib/registry-api.js"
	import { Pagination } from "bits-ui"
	import { formatDistanceToNow } from "date-fns"
	import { ChevronLeft, ChevronRight, SearchX } from "lucide-svelte"

	const { data } = $props()

	let displayDates = $state(false)

	$effect(() => {
		displayDates = true
	})
	console.log(data.result)
</script>

<div class="mx-auto max-w-screen-lg px-4">
	{#await data.result}
		<div class="sr-only">Loading...</div>
		<div
			class="my-8 flex flex-col sm:flex-row sm:items-center sm:justify-between"
			aria-hidden="true"
		>
			<div class="bg-card-hover w-20 animate-pulse rounded text-transparent">...</div>
			<div class="bg-card-hover mx-auto mt-8 h-8 w-48 animate-pulse rounded sm:m-0"></div>
		</div>
		<div class="mb-8 space-y-4">
			{#each Array.from({ length: 10 }).map((_, i) => i) as i}
				<div class="bg-card/50 overflow-hidden rounded px-5 py-4">
					<div class="mb-1 flex items-center justify-between">
						<div class="bg-card-hover w-52 animate-pulse rounded text-xl text-transparent">...</div>
						<div
							class="bg-card-hover hidden w-32 animate-pulse rounded text-sm text-transparent sm:block"
						>
							...
						</div>
					</div>

					<div
						class="bg-card-hover mb-3 w-96 max-w-full animate-pulse rounded text-sm text-transparent"
					>
						...
					</div>

					<div class="bg-card-hover w-16 max-w-full animate-pulse rounded text-sm text-transparent">
						...
					</div>
				</div>
			{/each}
		</div>
		<div class="bg-card-hover mx-auto my-8 h-8 w-48 animate-pulse rounded"></div>
	{:then result}
		{#if result.data.length === 0}
			<div class="flex flex-col items-center py-32 text-center">
				<SearchX class="mb-6 size-16" />
				<h1 class="text-heading font-bold">No results</h1>
				<p class="text-balance">We didn't find any packages matching your search query.</p>
			</div>
		{:else}
			{#snippet pagination()}
				<Pagination.Root
					count={result.count}
					page={data.page}
					perPage={data.pageSize}
					onPageChange={(page) => {
						const params = new URLSearchParams()
						params.set("q", data.query)
						params.set("page", page.toString())

						goto(`/search?${params}`)
					}}
					let:pages
				>
					<div class="flex items-center space-x-1">
						<Pagination.PrevButton
							class="hover:enabled:bg-card-hover inline-flex size-8 items-center justify-center rounded transition disabled:opacity-50"
						>
							<ChevronLeft />
						</Pagination.PrevButton>
						{#each pages as page (page.key)}
							{#if page.type === "ellipsis"}
								<div class="px-2">...</div>
							{:else}
								<Pagination.Page
									{page}
									class="hover:bg-card-hover hover:text-heading data-[selected]:bg-primary-bg data-[selected]:text-primary-fg inline-flex size-8 items-center justify-center rounded font-bold transition"
								>
									{page.value}
								</Pagination.Page>
							{/if}
						{/each}
						<Pagination.NextButton
							class="hover:enabled:bg-card-hover inline-flex size-8 items-center justify-center rounded transition disabled:opacity-50"
						>
							<ChevronRight />
						</Pagination.NextButton>
					</div>
				</Pagination.Root>
			{/snippet}

			<div class="my-8 flex flex-col sm:flex-row sm:items-center sm:justify-between">
				<h1 class="font-bold">{result.count} {result.count > 1 ? "results" : "result"}</h1>
				<div class="mx-auto mt-8 sm:m-0">
					{@render pagination()}
				</div>
			</div>

			<div class="mb-8 space-y-4">
				{#each result.data as pkg}
					{@const [scope, name] = pkg.name.split("/")}

					{#snippet timeAndVersion()}
						<time datetime={pkg.published_at}>
							{#if displayDates}
								{formatDistanceToNow(new Date(pkg.published_at), { addSuffix: true })}
							{:else}
								...
							{/if}
						</time>
						<span>{" · "}</span>
						<span class="truncate">v{pkg.version}</span>
					{/snippet}

					<article
						class="bg-card hover:bg-card-hover relative overflow-hidden rounded px-5 py-4 transition"
					>
						<div class="mb-1 flex items-center justify-between">
							<h3 class="truncate text-xl font-semibold">
								<a
									href={`/packages/${pkg.name}`}
									class="after:absolute after:inset-0 after:content-['']"
								>
									<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
								</a>
							</h3>
							<div
								class="text-heading hidden text-sm font-semibold sm:block"
								class:invisible={!displayDates}
							>
								{@render timeAndVersion()}
							</div>
						</div>

						<p class="mb-3 h-[1lh] overflow-hidden truncate text-sm">{pkg.description}</p>

						<div
							class={`text-primary text-sm font-bold ${displayDates ? "" : "invisible sm:visible"}`}
						>
							{pkg.targets.map((target) => TARGET_KIND_DISPLAY_NAMES[target.kind]).join(", ")}
							<span class="sm:hidden">
								<span>{" · "}</span>
								{@render timeAndVersion()}
							</span>
						</div>
					</article>
				{/each}
			</div>

			<div class="mx-auto my-8 max-w-min">
				{@render pagination()}
			</div>
		{/if}
	{:catch error}
		<div class="mx-auto max-w-screen-xl px-4 py-32 text-center">
			<h1 class="text-heading mb-1 text-4xl font-bold">Error</h1>
			<p class="text-lg">{error.message}</p>
		</div>
	{/await}
</div>
