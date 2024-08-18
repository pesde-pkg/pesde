<script lang="ts">
	import { Check, Clipboard } from "lucide-svelte"

	type Props = {
		command: string
		class?: string
	}

	const { command, class: classname = "" }: Props = $props()

	let didCopy = $state(false)
</script>

<div class={`flex h-11 items-center overflow-hidden rounded border text-sm ${classname}`}>
	<code class="truncate px-4">{command}</code>
	<button
		class="ml-auto flex size-11 items-center justify-center border-l bg-card/40 hover:bg-card/60"
		onclick={() => {
			navigator.clipboard.writeText(command)

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
