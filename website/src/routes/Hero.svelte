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
	<h1 class="mb-3 text-2xl font-semibold text-heading md:mb-6 md:text-4xl lg:text-5xl">
		Manage your packages<br />
		<span class="sr-only"> for Luau</span>
		<span class="text-primary" aria-hidden="true">
			for {typewriteText}{#if cursorVisible}
				<span
					class="ml-1 inline-block h-[1.125rem] w-0.5 bg-current duration-100 md:h-7 lg:h-9"
					class:animate-cursor-blink={blink}
				></span>
			{/if}
		</span>
	</h1>

	<p class="mb-5 max-w-sm md:mb-8 md:max-w-md md:text-lg">
		A package manager for the Luau programming language, supporting multiple runtimes including
		Roblox and Lune.
	</p>

	<a
		href="/docs/get-started"
		class="bg-primary-bg inline-flex h-10 items-center rounded px-4 font-semibold text-primary-fg transition hover:bg-primary-hover md:h-11 md:px-5"
	>
		Get Started
	</a>
</section>
