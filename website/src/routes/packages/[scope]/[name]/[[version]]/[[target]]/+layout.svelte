<script lang="ts">
	import { formatDistanceToNow } from "date-fns"
	import { Check, ChevronDownIcon, Clipboard } from "lucide-svelte"
	import Tab from "./Tab.svelte"
	import { page } from "$app/stores"
	import { goto } from "$app/navigation"

	let { children, data } = $props()

	let didCopy = $state(false)

	const [scope, name] = data.pkg.name.split("/")

	const installCommand = `pesde add ${data.pkg.name}`

	const defaultTarget = $derived(
		"target" in $page.params ? $page.params.target : data.pkg.targets[0].kind,
	)

	const basePath = $derived.by(() => {
		const { scope, name } = $page.params
		if ("target" in $page.params) {
			const { version } = $page.params
			return `/packages/${scope}/${name}/${version}`
		}
		return `/packages/${scope}/${name}/latest`
	})
</script>

<div class="mx-auto flex max-w-screen-lg px-4 py-16">
	<div class="flex-grow pr-4">
		<h1 class="text-3xl font-bold">
			<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
		</h1>
		<div class="mb-2 font-semibold text-primary">
			v{data.pkg.version} ·
			<time
				datetime={data.pkg.published_at}
				title={new Date(data.pkg.published_at).toLocaleString()}
			>
				published {formatDistanceToNow(new Date(data.pkg.published_at), {
					addSuffix: true,
				})}
			</time>
		</div>
		<p class="mb-6 max-w-prose">{data.pkg.description}</p>

		<nav class="flex w-full border-b-2">
			<Tab tab="">Readme</Tab>
			<Tab tab="versions">Versions</Tab>
		</nav>

		{@render children()}
	</div>
	<aside class="ml-auto w-full max-w-[22rem] flex-shrink-0 border-l pl-4">
		<h2 class="mb-1 text-lg font-semibold text-heading">Install</h2>
		<div class="mb-4 flex h-11 items-center overflow-hidden rounded border text-sm">
			<code class="truncate px-4">{installCommand}</code>
			<button
				class="ml-auto flex size-11 items-center justify-center border-l bg-card/40 hover:bg-card/60"
				onclick={() => {
					navigator.clipboard.writeText(installCommand)

					if (didCopy) return

					didCopy = true
					setTimeout(() => {
						didCopy = false
					}, 1000)
				}}
			>
				{#if didCopy}
					<Check class="size-5" />
				{:else}
					<Clipboard class="size-5" />
				{/if}
			</button>
		</div>

		<h2 class="mb-1 text-lg font-semibold text-heading">
			<label for="target-select">Target</label>
		</h2>
		<div
			class="relative flex h-11 w-full items-center rounded border border-input-border bg-input-bg ring-0 ring-primary-bg/20 transition focus-within:border-primary focus-within:ring-4 has-[:disabled]:opacity-50"
		>
			<select
				class="absolute inset-0 appearance-none bg-transparent px-4 outline-none"
				id="target-select"
				onchange={(e) => {
					const select = e.currentTarget

					select.disabled = true
					goto(`${basePath}/${e.currentTarget.value}`).finally(() => {
						select.disabled = false
					})
				}}
			>
				{#each data.pkg.targets as target}
					<option value={target.kind} class="bg-card" selected={target.kind === defaultTarget}>
						{target.kind[0].toUpperCase() + target.kind.slice(1)}
					</option>
				{/each}
			</select>
			<ChevronDownIcon class="pointer-events-none absolute right-4 h-5 w-5" />
		</div>
	</aside>
</div>
