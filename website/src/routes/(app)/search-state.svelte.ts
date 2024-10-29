let _searchQuery = $state("")

export const searchQuery = {
	get value() {
		return _searchQuery
	},
	set value(value: string) {
		_searchQuery = value
	},
}
