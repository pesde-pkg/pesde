-- this table is purely for storage optimisation - PQ keys are much longer than 32 byte ed25519
CREATE TABLE public_key (
	id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,

	algorithm ENUM('ed25519') NOT NULL,
	data VARBINARY(4096) NOT NULL,

	blake3 BINARY(32) NOT NULL,

	UNIQUE (algorithm, blake3)
);


CREATE TABLE plaintext_name (
	blake3 BINARY(32) PRIMARY KEY,
	plaintext VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL
);


CREATE TABLE deprecation_reason (
	id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
	hash VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE,
	plaintext VARCHAR(255) NOT NULL
);


CREATE TABLE global_log (
	id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
	size BIGINT UNSIGNED NOT NULL
);


INSERT INTO global_log (size) VALUES (0);


CREATE TABLE global_log_node (
	pos BIGINT UNSIGNED PRIMARY KEY,
	blake3 BINARY(32) NOT NULL
);


CREATE TABLE global_log_entry (
	pos BIGINT UNSIGNED PRIMARY KEY,

	op_kind ENUM('scope_genesis') NOT NULL,

	published_at DATETIME(6) NOT NULL DEFAULT(UTC_TIMESTAMP(6)),

	UNIQUE (pos, op_kind),
	FOREIGN KEY (pos) REFERENCES global_log_node (pos)
);


CREATE TABLE global_log_scope_genesis_entry (
	pos BIGINT UNSIGNED PRIMARY KEY,

	op_kind ENUM('scope_genesis') NOT NULL DEFAULT 'scope_genesis' CHECK (op_kind = 'scope_genesis'),

	sig_algorithm ENUM('ssh_ed25519_sha512') NOT NULL,
	sig BLOB NOT NULL,

	name_blake3 BINARY(32) NOT NULL UNIQUE,

	owner_id BIGINT UNSIGNED NOT NULL,
	first_entry_hash VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

	UNIQUE (pos, op_kind),
	FOREIGN KEY (pos, op_kind) REFERENCES global_log_entry (pos, op_kind),
	FOREIGN KEY (name_blake3) REFERENCES plaintext_name (blake3),
	FOREIGN KEY (owner_id) REFERENCES public_key (id)
);


CREATE TABLE scope (
	genesis_pos BIGINT UNSIGNED PRIMARY KEY,

	log_size BIGINT UNSIGNED NOT NULL,

	FOREIGN KEY (genesis_pos) REFERENCES global_log_scope_genesis_entry (pos)
);


CREATE TABLE scope_log_node (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope) REFERENCES scope (genesis_pos)
);


CREATE TABLE package (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,

	local_name_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (scope, pos),
	UNIQUE (scope, local_name_blake3),
	FOREIGN KEY (scope, pos) REFERENCES scope_log_node (scope, pos),
	FOREIGN KEY (local_name_blake3) REFERENCES plaintext_name (blake3)
);


CREATE TABLE package_version (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	package_pos BIGINT UNSIGNED NOT NULL,

	version VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
	archive_hash VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

	PRIMARY KEY (scope, pos),
	UNIQUE (scope, package_pos, version),
	FOREIGN KEY (scope, pos) REFERENCES scope_log_node (scope, pos),
	FOREIGN KEY (scope, package_pos) REFERENCES package (scope, pos)
);


CREATE TABLE member_grant (
	blake3 BINARY(32) PRIMARY KEY
);


CREATE TABLE member_grant_package (
	grant_blake3 BINARY(32) NOT NULL,
	local_name_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (grant_blake3, local_name_blake3),
	FOREIGN KEY (grant_blake3) REFERENCES member_grant (blake3),
	FOREIGN KEY (local_name_blake3) REFERENCES plaintext_name (blake3)
);


CREATE TABLE scope_members_node (
	blake3 BINARY(32) PRIMARY KEY,
	kind ENUM('leaf', 'internal') NOT NULL,

	UNIQUE (blake3, kind)
);


CREATE TABLE scope_members_leaf_node_entry (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'leaf' CHECK (kind = 'leaf'),
	idx SMALLINT UNSIGNED NOT NULL,

	member BIGINT UNSIGNED NOT NULL,
	grant_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES scope_members_node (blake3, kind),
	FOREIGN KEY (member) REFERENCES public_key (id),
	FOREIGN KEY (grant_blake3) REFERENCES member_grant (blake3)
);


CREATE TABLE scope_members_internal_node_key (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	member BIGINT UNSIGNED NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES scope_members_node (blake3, kind),
	FOREIGN KEY (member) REFERENCES public_key (id)
);


CREATE TABLE scope_members_internal_node_child (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	child_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES scope_members_node (blake3, kind),
	FOREIGN KEY (child_blake3) REFERENCES scope_members_node (blake3)
);


CREATE TABLE versions_node (
	blake3 BINARY(32) PRIMARY KEY,
	kind ENUM('leaf', 'internal') NOT NULL,

	UNIQUE (blake3, kind)
);


CREATE TABLE versions_leaf_node_entry (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'leaf' CHECK (kind = 'leaf'),
	idx SMALLINT UNSIGNED NOT NULL,

	scope BIGINT UNSIGNED NOT NULL,
	version_pos BIGINT UNSIGNED NOT NULL,
	yank_state ENUM('yanked', 'admin_yanked'),

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES versions_node (blake3, kind),
	FOREIGN KEY (scope, version_pos) REFERENCES package_version (scope, pos)
);


CREATE TABLE versions_internal_node_key (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	scope BIGINT UNSIGNED NOT NULL,
	version_pos BIGINT UNSIGNED NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES versions_node (blake3, kind),
	FOREIGN KEY (scope, version_pos) REFERENCES package_version (scope, pos)
);


CREATE TABLE versions_internal_node_child (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	child_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES versions_node (blake3, kind),
	FOREIGN KEY (child_blake3) REFERENCES versions_node (blake3)
);


CREATE TABLE deprecations_node (
	blake3 BINARY(32) PRIMARY KEY,
	kind ENUM('leaf', 'internal') NOT NULL,

	UNIQUE (blake3, kind)
);


CREATE TABLE deprecations_leaf_node_entry (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'leaf' CHECK (kind = 'leaf'),
	idx SMALLINT UNSIGNED NOT NULL,

	scope BIGINT UNSIGNED NOT NULL,
	package_pos BIGINT UNSIGNED NOT NULL,
	reason_id BIGINT UNSIGNED NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES deprecations_node (blake3, kind),
	FOREIGN KEY (scope, package_pos) REFERENCES package (scope, pos),
	FOREIGN KEY (reason_id) REFERENCES deprecation_reason (id)
);


CREATE TABLE deprecations_internal_node_key (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	scope BIGINT UNSIGNED NOT NULL,
	package_pos BIGINT UNSIGNED NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES deprecations_node (blake3, kind),
	FOREIGN KEY (scope, package_pos) REFERENCES package (scope, pos)
);


CREATE TABLE deprecations_internal_node_child (
	node_blake3 BINARY(32) NOT NULL,
	kind ENUM('leaf', 'internal') NOT NULL DEFAULT 'internal' CHECK (kind = 'internal'),
	idx SMALLINT UNSIGNED NOT NULL,

	child_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (node_blake3, idx),
	FOREIGN KEY (node_blake3, kind) REFERENCES deprecations_node (blake3, kind),
	FOREIGN KEY (child_blake3) REFERENCES deprecations_node (blake3)
);


CREATE TABLE scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,

	ns ENUM('signed', 'admin') NOT NULL,
	published_at DATETIME(6) NOT NULL DEFAULT(UTC_TIMESTAMP(6)),
	prev_hash VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

	owner BIGINT UNSIGNED NOT NULL,
	scope_members_root_blake3 BINARY(32) NOT NULL,
	versions_root_blake3 BINARY(32) NOT NULL,
	deprecations_root_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (scope, pos),
	UNIQUE (scope, pos, ns),
	FOREIGN KEY (scope, pos) REFERENCES scope_log_node (scope, pos),
	FOREIGN KEY (owner) REFERENCES public_key (id),
	FOREIGN KEY (scope_members_root_blake3) REFERENCES scope_members_node (blake3),
	FOREIGN KEY (versions_root_blake3) REFERENCES versions_node (blake3),
	FOREIGN KEY (deprecations_root_blake3) REFERENCES deprecations_node (blake3)
);


CREATE TABLE signed_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	ns ENUM('signed', 'admin') NOT NULL DEFAULT 'signed' CHECK (ns = 'signed'),
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL,
	signer BIGINT UNSIGNED NOT NULL,

	sig_algorithm ENUM('ssh_ed25519_sha512') NOT NULL,
	sig BLOB NOT NULL,

	PRIMARY KEY (scope, pos),
	UNIQUE (scope, pos, op_kind),
	FOREIGN KEY (scope, pos, ns) REFERENCES scope_log_entry (scope, pos, ns),
	FOREIGN KEY (signer) REFERENCES public_key (id)
);


CREATE TABLE signed_add_member_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'add_member' CHECK (op_kind = 'add_member'),

	member BIGINT UNSIGNED NOT NULL,
	grant_blake3 BINARY(32) NOT NULL,

	consent_algorithm ENUM('ssh_ed25519_sha512') NOT NULL,
	consent BLOB NOT NULL,
	nonce BINARY(16) NOT NULL UNIQUE,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (member) REFERENCES public_key (id),
	FOREIGN KEY (grant_blake3) REFERENCES member_grant (blake3)
);


CREATE TABLE signed_update_member_grant_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'update_member_grant' CHECK (op_kind = 'update_member_grant'),

	member BIGINT UNSIGNED NOT NULL,
	grant_blake3 BINARY(32) NOT NULL,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (member) REFERENCES public_key (id),
	FOREIGN KEY (grant_blake3) REFERENCES member_grant (blake3)
);


CREATE TABLE signed_rotate_key_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'rotate_key' CHECK (op_kind = 'rotate_key'),

	new_key BIGINT UNSIGNED NOT NULL,

	consent_algorithm ENUM('ssh_ed25519_sha512') NOT NULL,
	consent BLOB NOT NULL,
	nonce BINARY(16) NOT NULL UNIQUE,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (new_key) REFERENCES public_key (id)
);


CREATE TABLE signed_remove_member_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'remove_member' CHECK (op_kind = 'remove_member'),

	member BIGINT UNSIGNED,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (member) REFERENCES public_key (id)
);


CREATE TABLE signed_transfer_ownership_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'transfer_ownership' CHECK (op_kind = 'transfer_ownership'),

	consent_algorithm ENUM('ssh_ed25519_sha512') NOT NULL,
	consent BLOB NOT NULL,
	nonce BINARY(16) NOT NULL UNIQUE,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind)
);


CREATE TABLE signed_publish_version_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'publish_version' CHECK (op_kind = 'publish_version'),

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (scope, pos) REFERENCES package_version (scope, pos)
);


CREATE TABLE signed_set_yanked_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'set_yanked' CHECK (op_kind = 'set_yanked'),

	version_pos BIGINT UNSIGNED NOT NULL,
	yanked BOOLEAN NOT NULL,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (scope, version_pos) REFERENCES package_version (scope, pos)
);


CREATE TABLE signed_set_deprecation_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM(
		'add_member',
		'update_member_grant',
		'rotate_key',
		'remove_member',
		'transfer_ownership',
		'publish_version',
		'set_yanked',
		'set_deprecation'
	) NOT NULL DEFAULT 'set_deprecation' CHECK (op_kind = 'set_deprecation'),

	package_pos BIGINT UNSIGNED NOT NULL,
	reason_id BIGINT UNSIGNED,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES signed_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (scope, package_pos) REFERENCES package (scope, pos),
	FOREIGN KEY (reason_id) REFERENCES deprecation_reason (id)
);


CREATE TABLE admin_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	ns ENUM('signed', 'admin') NOT NULL DEFAULT 'admin' CHECK (ns = 'admin'),
	op_kind ENUM('transfer_ownership', 'set_yanked') NOT NULL,

	PRIMARY KEY (scope, pos),
	UNIQUE (scope, pos, op_kind),
	FOREIGN KEY (scope, pos, ns) REFERENCES scope_log_entry (scope, pos, ns)
);


CREATE TABLE admin_transfer_ownership_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM('transfer_ownership', 'set_yanked') NOT NULL DEFAULT 'transfer_ownership' CHECK (op_kind = 'transfer_ownership'),

	new_owner BIGINT UNSIGNED NOT NULL,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES admin_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (new_owner) REFERENCES public_key (id)
);


CREATE TABLE admin_set_yanked_scope_log_entry (
	scope BIGINT UNSIGNED NOT NULL,
	pos BIGINT UNSIGNED NOT NULL,
	op_kind ENUM('transfer_ownership', 'set_yanked') NOT NULL DEFAULT 'set_yanked' CHECK (op_kind = 'set_yanked'),

	version_pos BIGINT UNSIGNED NOT NULL,
	yanked BOOLEAN NOT NULL,

	PRIMARY KEY (scope, pos),
	FOREIGN KEY (scope, pos, op_kind) REFERENCES admin_scope_log_entry (scope, pos, op_kind),
	FOREIGN KEY (scope, version_pos) REFERENCES package_version (scope, pos)
);