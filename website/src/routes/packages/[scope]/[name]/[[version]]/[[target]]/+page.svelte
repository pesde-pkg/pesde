<script>
	const { data } = $props()
</script>

<div class="prose max-w-none py-8">
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html data.readmeHtml}
</div>
