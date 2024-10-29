<script lang="ts">
	import { Search } from "lucide-svelte"
	import { searchQuery } from "./search-state.svelte"
</script>

<form action="/search" class="w-full">
	<label
		class="relative flex h-9 w-full items-center overflow-hidden rounded border border-input-border bg-input-bg px-2 outline-none ring-0 ring-primary-bg/20 transition focus-within:border-primary focus-within:ring-4"
	>
		<Search class="size-5" aria-label="search" />
		<input
			name="q"
			placeholder="search packages..."
			class="absolute inset-0 bg-transparent pl-9 pr-2 text-heading placeholder:text-placeholder"
			bind:value={searchQuery.value}
		/>
	</label>
</form>
