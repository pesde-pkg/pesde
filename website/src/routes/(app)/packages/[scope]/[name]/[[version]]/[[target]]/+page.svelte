<script>
	const { data } = $props()
</script>

<div
	class="prose prose-pre:w-full prose-pre:overflow-auto prose-img:inline-block prose-img:m-0 prose-video:inline-block prose-video:m-0 min-w-0 py-8"
>
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
	{@html data.readmeHtml}
</div>
