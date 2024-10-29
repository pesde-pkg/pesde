<script module lang="ts">
	let _activeHeaderId = $state("")

	export const activeHeaderId = {
		get value() {
			return _activeHeaderId
		},

		set value(value) {
			_activeHeaderId = value
		},
	}
</script>

<script lang="ts">
	import type { TocItem } from "$lib/markdown"

	type Props = {
		toc: TocItem[]
	}

	const { toc }: Props = $props()

	$effect(() => {
		toc

		const isHeading = (el: Element): boolean => {
			if (el.id === "_top") return true
			if (el instanceof HTMLHeadingElement) {
				const level = el.tagName[1]
				if (level) {
					const int = parseInt(level, 10)
					if (int >= 2 && int <= 3) return true
				}
			}
			return false
		}

		const getNearestHeading = (el: Element | null): Element | null => {
			if (!el) return null

			while (el) {
				if (isHeading(el)) return el
				el = el.previousElementSibling
			}

			return null
		}

		const callback: IntersectionObserverCallback = (entries) => {
			for (const entry of entries) {
				if (!entry.isIntersecting) continue

				const heading = getNearestHeading(entry.target)
				if (!heading) continue

				const item = toc.find((item) => {
					if (item.id === "_top" && heading.id === "_top") return true
					return `user-content-${item.id}` === heading.id
				})
				if (item) {
					activeHeaderId.value = item.id
				}
			}
		}

		const observer = new IntersectionObserver(callback, {
			rootMargin: "-56px 0px -80% 0px",
		})

		const targets = document.querySelectorAll("main > *")
		for (const target of targets) {
			observer.observe(target)
		}

		return () => {
			observer.disconnect()
		}
	})
</script>
