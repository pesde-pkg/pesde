<script lang="ts">
	const { data } = $props()
</script>

<main class="prose max-w-none py-16">
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html data.html}
</main>
