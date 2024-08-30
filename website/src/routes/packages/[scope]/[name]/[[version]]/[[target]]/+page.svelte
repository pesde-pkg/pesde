<script>
	const { data } = $props()
</script>

<div class="prose min-w-0 py-8 prose-pre:w-full prose-pre:overflow-auto">
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html data.readmeHtml}
</div>
