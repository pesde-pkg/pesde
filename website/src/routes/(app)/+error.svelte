<script>
	import { page } from "$app/stores"
</script>

<div class="mx-auto max-w-screen-xl px-4 py-32 text-center">
	<h1 class="mb-1 text-4xl font-bold text-heading">{$page.status}</h1>
	<p class="text-lg">{$page.error?.message}</p>
</div>
