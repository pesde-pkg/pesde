<script lang="ts">
	import { onMount } from "svelte"

	export const prerender = true

	const tools = ["Luau", "Roblox", "Lune"]

	let typewriteText = $state("Luau")
	let blink = $state(true)
	let cursorVisible = $state(false)

	onMount(() => {
		let current = 0
		let timeout: number

		function typewrite(text: string) {
			blink = false

			let progress = 0

			timeout = setInterval(() => {
				progress++
				typewriteText = text.slice(0, progress)

				if (progress >= text.length) {
					blink = true

					clearInterval(timeout)
					timeout = setTimeout(() => clear(), 3500)
				}
			}, 120)
		}

		function clear() {
			blink = false

			let progress = typewriteText.length

			timeout = setInterval(() => {
				progress--
				typewriteText = typewriteText.slice(0, progress)

				if (progress <= 0) {
					clearInterval(timeout)
					timeout = setTimeout(() => {
						current++
						if (current >= tools.length) current = 0
						typewrite(tools[current])
					}, 1000)
				}
			}, 80)
		}

		cursorVisible = true
		timeout = setTimeout(() => {
			clear()
		}, 4500)

		return () => {
			clearTimeout(timeout)
		}
	})
</script>

<section class="mx-auto max-w-screen-xl px-4 py-32">
	<h1 class="mb-6 text-5xl font-semibold text-heading">
		Manage your packages<br />
		<span class="sr-only"> for Luau</span>
		<span class="text-primary" aria-hidden="true">
			for {typewriteText}{#if cursorVisible}
				<span
					class="ml-1 inline-block h-9 w-0.5 bg-current duration-100"
					class:animate-cursor-blink={blink}
				></span>
			{/if}
		</span>
	</h1>

	<p class="mb-8 max-w-md text-lg">
		A package manager for the Luau programming language, supporting multiple runtimes including
		Roblox and Lune.
	</p>

	<a
		href="#"
		class="hover:bg-primary-hover inline-flex h-11 items-center rounded bg-primary px-5 font-semibold text-primary-fg transition"
	>
		Get Started
	</a>
</section>
