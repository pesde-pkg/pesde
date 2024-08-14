<script lang="ts">
	import { formatDistanceToNow } from "date-fns"
	import { Check, Clipboard } from "lucide-svelte"
	import Tab from "./Tab.svelte"

	let { children, data } = $props()

	let didCopy = $state(false)

	const [scope, name] = data.pkg.name.split("/")

	const installCommand = `pesde add ${data.pkg.name}`
</script>

<div class="mx-auto flex max-w-screen-xl px-4 py-16">
	<div class="flex-grow pr-4">
		<h1 class="text-3xl font-bold">
			<span class="text-heading">{scope}/</span><span class="text-light">{name}</span>
		</h1>
		<div class="mb-2 font-semibold text-primary">
			v{data.pkg.version} · published {formatDistanceToNow(new Date(data.pkg.published_at), {
				addSuffix: true,
			})}
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
		<div class="flex h-11 items-center overflow-hidden rounded border text-sm">
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
	</aside>
</div>
